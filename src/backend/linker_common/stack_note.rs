//! `.note.GNU-stack` policy shared by every output writer.
//!
//! The rule is bfd's, and it is the only rule that keeps nested-function
//! trampolines working without silently widening every stack:
//!
//! * a command-line `-z execstack` / `-z noexecstack` decides the output
//!   `PT_GNU_STACK` outright, and suppresses the input scan entirely;
//! * otherwise the output stack is executable exactly when *some* input marked
//!   its `.note.GNU-stack` section `SHF_EXECINSTR`.
//!
//! Keeping the scan here (rather than inlined in each writer) means the
//! executable and shared-library paths cannot disagree about the answer, and
//! that the warning a user sees always describes the header that was emitted.

use crate::backend::elf::SHF_EXECINSTR;
use crate::backend::linker_common::types::Elf64Object;

/// `.note.GNU-stack` is a zero-sized marker whose *flags*, not contents,
/// carry the request.
const GNU_STACK_NOTE: &str = ".note.GNU-stack";

/// Does this single input ask for an executable stack?
fn input_requires_exec_stack(obj: &Elf64Object) -> bool {
    obj.sections
        .iter()
        .any(|sec| sec.name == GNU_STACK_NOTE && sec.flags & SHF_EXECINSTR != 0)
}

/// The inputs that ask for an executable stack, in link order.
///
/// Used for bfd-compatible diagnostics: bfd names every object that forces the
/// whole program's stack to RWX, because that is a security decision the user
/// did not necessarily make.
pub fn requiring_inputs(objects: &[Elf64Object]) -> Vec<&str> {
    objects
        .iter()
        .filter(|obj| input_requires_exec_stack(obj))
        .map(|obj| obj.source_name.as_str())
        .collect()
}

/// The `PT_GNU_STACK` executability implied by the inputs alone.
///
/// Call this only when the command line did not decide: `-z execstack` and
/// `-z noexecstack` override this answer.
pub fn requires_exec_stack(objects: &[Elf64Object]) -> bool {
    objects.iter().any(input_requires_exec_stack)
}

/// The bfd-compatible warning line for one input, or `None` when the input
/// does not force an executable stack.
pub fn warning_for(input: &str) -> String {
    format!(
        "{input}: requires executable stack (because the {GNU_STACK_NOTE} section is executable)"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::linker_common::SectionData;
    use crate::backend::linker_common::types::Elf64Section;

    fn section(name: &str, flags: u64) -> Elf64Section {
        Elf64Section {
            name_idx: 0,
            name: name.to_string(),
            sh_type: 1,
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

    fn object(name: &str, sections: Vec<Elf64Section>) -> Elf64Object {
        Elf64Object {
            sections,
            symbols: Vec::new(),
            section_data: Vec::<SectionData>::new(),
            relocations: Vec::new(),
            source_name: name.to_string(),
        }
    }

    #[test]
    fn absent_note_is_not_executable() {
        let obj = object("a.o", vec![section(".text", SHF_EXECINSTR)]);
        assert!(!requires_exec_stack(std::slice::from_ref(&obj)));
        assert!(requiring_inputs(std::slice::from_ref(&obj)).is_empty());
    }

    #[test]
    fn plain_note_is_not_executable() {
        let obj = object("a.o", vec![section(".note.GNU-stack", 0)]);
        assert!(!requires_exec_stack(std::slice::from_ref(&obj)));
    }

    #[test]
    fn executable_note_is_reported_by_name() {
        let obj = object("tramp.o", vec![section(".note.GNU-stack", SHF_EXECINSTR)]);
        assert!(requires_exec_stack(std::slice::from_ref(&obj)));
        assert_eq!(requiring_inputs(std::slice::from_ref(&obj)), ["tramp.o"]);
        assert_eq!(
            warning_for("tramp.o"),
            "tramp.o: requires executable stack \
             (because the .note.GNU-stack section is executable)"
        );
    }

    #[test]
    fn exec_flag_on_another_section_does_not_count() {
        // Only the marker section's flag means anything; a `.text` that
        // happens to be executable is the normal case and must not leak into
        // the stack decision.
        let obj = object(
            "a.o",
            vec![
                section(".text", SHF_EXECINSTR | 2),
                section(".note.GNU-stack", 0),
            ],
        );
        assert!(!requires_exec_stack(std::slice::from_ref(&obj)));
    }

    #[test]
    fn every_forcing_input_is_named_in_link_order() {
        let objs = vec![
            object("a.o", vec![section(".note.GNU-stack", 0)]),
            object("b.o", vec![section(".note.GNU-stack", SHF_EXECINSTR)]),
            object("c.o", vec![section(".note.GNU-stack", SHF_EXECINSTR)]),
        ];
        assert!(requires_exec_stack(&objs));
        assert_eq!(requiring_inputs(&objs), ["b.o", "c.o"]);
    }
}
