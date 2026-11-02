//! `.note.GNU-stack` policy shared by every output writer: the
//! `PT_GNU_STACK` decision and the executable-stack diagnostics, both as GNU
//! ld 2.47 makes them (`bfd_elf_size_dynamic_sections` in `bfd/elflink.c`).
//!
//! * `-z execstack` / `-z noexecstack` decide the header outright and
//!   suppress the input scan;
//! * otherwise the inputs' markers decide: the stack is executable when some
//!   input's `.note.GNU-stack` is `SHF_EXECINSTR` (a nested-function
//!   trampoline), and without any marker GNU ld emits no header at all. On
//!   targets whose ABI defaults to an executable stack (`default_execstack`)
//!   an input without a marker counts as asking for one.
//!
//! The diagnostics follow the same code: the FIRST input with an executable
//! marker is named (the scan stops there), `--no-warn-execstack` silences
//! it, `-z execstack` is reported only under an explicit `--warn-execstack`,
//! and `--error-execstack` turns either report into GNU ld's error.
//!
//! Keeping both here means the executable and shared-library writers of every
//! target cannot disagree about the answer, and that a diagnostic always
//! describes the header that was emitted. (The x86-64 writers keep their
//! long-standing always-present header — RW unless something forces RWX —
//! which differs from GNU ld only for links in which no input carries a
//! marker, and is the safer of the two answers there.)

use super::types::Elf64Object;
use crate::backend::elf::{PF_R, PF_W, PF_X, SHF_EXECINSTR, SHT_NULL};

/// `.note.GNU-stack` is a zero-sized marker whose *flags*, not contents,
/// carry the request.
const GNU_STACK_NOTE: &str = ".note.GNU-stack";

/// What one input says about the stack; `None` for an input without
/// sections, which GNU ld skips.
fn marker(obj: &Elf64Object) -> Option<Option<bool>> {
    if obj.sections.iter().all(|s| s.sh_type == SHT_NULL) {
        return None;
    }
    Some(
        obj.sections
            .iter()
            .find(|s| s.name == GNU_STACK_NOTE)
            .map(|note| note.flags & SHF_EXECINSTR != 0),
    )
}

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
        match marker(obj) {
            None => {}
            Some(Some(x)) => {
                marked = true;
                if x {
                    exec = true;
                    break;
                }
            }
            Some(None) => exec |= default_execstack,
        }
    }
    marked.then_some(PF_R | PF_W | if exec { PF_X } else { 0 })
}

/// The `PT_GNU_STACK` executability implied by the inputs alone, for writers
/// that always emit the header.
///
/// Call this only when the command line did not decide: `-z execstack` and
/// `-z noexecstack` override this answer.
pub fn requires_exec_stack(objects: &[Elf64Object]) -> bool {
    gnu_stack_flags(None, false, objects).is_some_and(|f| f & PF_X != 0)
}

/// The executable-stack options of one link.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExecstackOptions {
    /// `-z execstack` (`Some(true)`) / `-z noexecstack` (`Some(false)`).
    pub z_execstack: Option<bool>,
    /// `--warn-execstack` (`Some(true)`) / `--no-warn-execstack`
    /// (`Some(false)`); `None` is GNU ld's default and
    /// `--warn-execstack-objects`: report executable markers only.
    pub warn: Option<bool>,
    /// `--error-execstack`: a report is an error, not a warning.
    pub error: bool,
}

/// One executable-stack report: GNU ld's text without its `warning: ` /
/// `error: ` prefix.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExecstackReport {
    Warning(String),
    Error(String),
}

/// GNU ld's executable-stack report for this link, or `None` when it prints
/// none. `objects` pairs each input with the name GNU ld would print, in link
/// order.
pub fn execstack_report<'a>(
    opts: ExecstackOptions,
    objects: impl IntoIterator<Item = (&'a str, &'a Elf64Object)>,
) -> Option<ExecstackReport> {
    let report = |warning: String, error: String| {
        Some(if opts.error {
            ExecstackReport::Error(error)
        } else {
            ExecstackReport::Warning(warning)
        })
    };
    match opts.z_execstack {
        Some(true) if opts.warn == Some(true) => {
            return report(
                "enabling an executable stack because of -z execstack command line option".into(),
                "creating an executable stack because of -z execstack command line option".into(),
            );
        }
        Some(_) => return None,
        None if opts.warn == Some(false) => return None,
        None => {}
    }
    let name = objects
        .into_iter()
        .find_map(|(name, obj)| (marker(obj) == Some(Some(true))).then_some(name))?;
    report(
        format!(
            "{name}: requires executable stack (because the {GNU_STACK_NOTE} section is executable)"
        ),
        format!(
            "{name}: is triggering the generation of an executable stack (because it has an executable {GNU_STACK_NOTE} section)"
        ),
    )
}

/// Print [`execstack_report`]'s warning the way every lccc-ld diagnostic is
/// printed, or return its error (`--error-execstack`, or a warning under
/// `--fatal-warnings`) for the caller to fail the link with.
pub fn report_execstack<'a>(
    opts: ExecstackOptions,
    fatal_warnings: bool,
    objects: impl IntoIterator<Item = (&'a str, &'a Elf64Object)>,
) -> Result<(), String> {
    match execstack_report(opts, objects) {
        None => Ok(()),
        Some(ExecstackReport::Error(e)) => Err(e),
        Some(ExecstackReport::Warning(w)) if fatal_warnings => {
            Err(format!("warning treated as error: {w}"))
        }
        Some(ExecstackReport::Warning(w)) => {
            eprintln!("lccc-ld: warning: {w}");
            Ok(())
        }
    }
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
            sections.push(section(GNU_STACK_NOTE, SHT_PROGBITS, flags));
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

    #[test]
    fn only_the_marker_section_counts_for_always_present_headers() {
        // A `.text` is executable in every object; only the marker's flag
        // means anything, and an absent marker is not executable.
        assert!(!requires_exec_stack(&[object(None)]));
        assert!(!requires_exec_stack(&[object(Some(0))]));
        assert!(requires_exec_stack(&[
            object(Some(0)),
            object(Some(SHF_EXECINSTR))
        ]));
    }

    /// GNU ld 2.47's reports (`elflink.c`; the warnings also observed with the
    /// pinned RISC-V ld): the scan stops at the FIRST executable marker, so
    /// one input is named even when several force the stack.
    #[test]
    fn execstack_reports_match_gnu_ld() {
        let (plain, exec1, exec2) = (
            object(Some(0)),
            object(Some(SHF_EXECINSTR)),
            object(Some(SHF_EXECINSTR)),
        );
        let empty = with_sections(vec![section("", SHT_NULL, 0)]);
        let inputs = [("plain.o", &plain), ("x1.o", &exec1), ("x2.o", &exec2)];
        let rep = |z_execstack, warn, error| {
            execstack_report(
                ExecstackOptions {
                    z_execstack,
                    warn,
                    error,
                },
                inputs,
            )
        };
        let w = |s: &str| Some(ExecstackReport::Warning(s.to_string()));
        let e = |s: &str| Some(ExecstackReport::Error(s.to_string()));
        let named =
            "x1.o: requires executable stack (because the .note.GNU-stack section is executable)";
        assert_eq!(rep(None, None, false), w(named));
        assert_eq!(rep(None, Some(true), false), w(named));
        assert_eq!(rep(None, Some(false), false), None);
        assert_eq!(rep(None, Some(false), true), None);
        assert_eq!(
            rep(None, None, true),
            e("x1.o: is triggering the generation of an executable stack \
               (because it has an executable .note.GNU-stack section)")
        );
        assert_eq!(rep(Some(false), Some(true), true), None);
        assert_eq!(rep(Some(true), None, true), None);
        assert_eq!(
            rep(Some(true), Some(true), false),
            w("enabling an executable stack because of -z execstack command line option")
        );
        assert_eq!(
            rep(Some(true), Some(true), true),
            e("creating an executable stack because of -z execstack command line option")
        );
        // Non-executable markers, unmarked and section-less inputs are silent.
        let quiet = object(None);
        assert_eq!(
            execstack_report(
                ExecstackOptions::default(),
                [("p", &plain), ("q", &quiet), ("e", &empty)]
            ),
            None
        );
    }

    #[test]
    fn fatal_warnings_promote_and_errors_fail() {
        let exec = object(Some(SHF_EXECINSTR));
        let one = [("t.o", &exec)];
        let opts = ExecstackOptions::default();
        assert_eq!(
            report_execstack(opts, true, one),
            Err("warning treated as error: t.o: requires executable stack \
                 (because the .note.GNU-stack section is executable)"
                .to_string())
        );
        let error = ExecstackOptions {
            error: true,
            ..opts
        };
        assert!(report_execstack(error, false, one).is_err());
        let quiet = ExecstackOptions {
            warn: Some(false),
            ..opts
        };
        assert_eq!(report_execstack(quiet, true, one), Ok(()));
    }
}
