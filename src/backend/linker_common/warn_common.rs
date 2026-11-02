//! `--warn-common`: GNU ld's diagnostics for COMMON (tentative) symbols.
//!
//! The model replays the linked inputs in registration order and reproduces
//! the five `ldmain.c` messages byte-for-byte (modulo the program prefix),
//! including bfd's ownership rules, which differ from symbol resolution:
//!
//! - a strong definition takes ownership from a common, a shared-library
//!   definition included (`s1.o ./libs.so s2.o` blames `./libs.so` twice),
//!   but only a strong *data* definition of a library takes part: a weak or
//!   function export never meets a common (`shared_def_overrides_common`);
//! - a common never takes ownership from a definition;
//! - a regular definition silently replaces a shared-library owner;
//! - a larger common takes ownership from a smaller one, a smaller or equal
//!   one does not;
//! - once a regular object gives a name non-default visibility (a hidden
//!   common or reference), library definitions of it no longer take part,
//!   and a library owner is dropped without a message;
//! - weak definitions, undefined references and locals never participate.
//!
//! Archive members that were not extracted are not inputs, so a library
//! member whose symbol is already common stays silent, as in bfd. A library
//! fed again (a group rescan) is ignored. Every rule above was measured
//! against GNU ld (see the unit tests).
//!
//! Inputs are fed in the order the linker registers their symbols, which is
//! the order its own resolution sees them: `ObjectSet::push` for the ELF64
//! linkers' objects and `ObjectSet::note_linked_library` for the shared
//! libraries they keep, the command-line position for i686.  A library that
//! `--as-needed` drops is unloaded, symbols included, and is never fed.

use super::types::{DynSymbol, Elf64Object};
use crate::backend::elf::{STB_LOCAL, STT_FILE, STT_SECTION, STV_DEFAULT};
use crate::common::fx_hash::{FxHashMap, FxHashSet};

/// How one input contributes a global symbol to the `--warn-common` model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommonRole {
    /// A tentative definition of the given size (`st_size`).
    Common(u64),
    /// A strong (non-weak) definition.
    Definition,
}

/// One global symbol of one input, as the model sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommonSymbol<'a> {
    pub name: &'a str,
    /// `None` for a symbol that matters only through `restricted`.
    pub role: Option<CommonRole>,
    /// A regular object gives the name non-default visibility.
    pub restricted: bool,
}

#[derive(Clone, Copy, Debug)]
enum Owner {
    Common { size: u64, file: usize },
    Definition { file: usize, dynamic: bool },
}

/// Accumulates `--warn-common` diagnostics over inputs fed in link order.
#[derive(Debug, Default)]
pub struct CommonDiagnostics {
    files: Vec<String>,
    owners: FxHashMap<String, Owner>,
    /// Names a regular object gave non-default visibility.
    restricted: FxHashSet<String>,
    warnings: Vec<String>,
}

impl CommonDiagnostics {
    /// Feeds one input. `dynamic` marks a shared library, whose definitions
    /// yield ownership to a later regular definition.
    pub fn add_input<'a>(
        &mut self,
        file: &str,
        dynamic: bool,
        symbols: impl IntoIterator<Item = CommonSymbol<'a>>,
    ) {
        if dynamic && self.files.iter().any(|f| f == file) {
            return;
        }
        let new = self.files.len();
        self.files.push(file.to_string());
        for CommonSymbol {
            name,
            role,
            restricted,
        } in symbols
        {
            if dynamic {
                if self.restricted.contains(name) {
                    continue;
                }
            } else if restricted {
                if !self.restricted.contains(name) {
                    self.restricted.insert(name.to_string());
                }
                if let Some(Owner::Definition { dynamic: true, .. }) = self.owners.get(name) {
                    self.owners.remove(name);
                }
            }
            let Some(role) = role else {
                continue;
            };
            let Some(owner) = self.owners.get_mut(name) else {
                let owner = match role {
                    CommonRole::Common(size) => Owner::Common { size, file: new },
                    CommonRole::Definition => Owner::Definition { file: new, dynamic },
                };
                self.owners.insert(name.to_string(), owner);
                continue;
            };
            let files = &self.files;
            let warning = match (role, *owner) {
                (CommonRole::Definition, Owner::Common { file: old, .. }) => {
                    *owner = Owner::Definition { file: new, dynamic };
                    Some(format!(
                        "{}: warning: definition of `{name}' overriding common from {}",
                        files[new], files[old]
                    ))
                }
                (CommonRole::Definition, Owner::Definition { dynamic: true, .. }) if !dynamic => {
                    *owner = Owner::Definition {
                        file: new,
                        dynamic: false,
                    };
                    None
                }
                (CommonRole::Definition, Owner::Definition { .. }) => None,
                (CommonRole::Common(_), Owner::Definition { file: old, .. }) => Some(format!(
                    "{}: warning: common of `{name}' overridden by definition from {}",
                    files[new], files[old]
                )),
                (
                    CommonRole::Common(size),
                    Owner::Common {
                        size: old_size,
                        file: old,
                    },
                ) => {
                    if size > old_size {
                        *owner = Owner::Common { size, file: new };
                        Some(format!(
                            "{}: warning: common of `{name}' overriding smaller common from {}",
                            files[new], files[old]
                        ))
                    } else if size < old_size {
                        Some(format!(
                            "{}: warning: common of `{name}' overridden by larger common from {}",
                            files[new], files[old]
                        ))
                    } else {
                        Some(format!(
                            "{} and {}: warning: multiple common of `{name}'",
                            files[new], files[old]
                        ))
                    }
                }
            };
            self.warnings.extend(warning);
        }
    }

    /// Feeds a relocatable object's global symbols.
    pub fn add_object(&mut self, obj: &Elf64Object) {
        let symbols = obj.symbols.iter().filter_map(|sym| {
            object_symbol(
                &sym.name,
                sym.binding(),
                sym.sym_type(),
                sym.shndx,
                sym.size,
                sym.visibility(),
            )
        });
        self.add_input(&obj.source_name, false, symbols);
    }

    /// Feeds a shared library's exports from its `.dynsym` (`path` names it
    /// in the diagnostics, as GNU ld does). Only default-version definitions
    /// bind an unversioned name; a hidden `name@VER` never meets a common.
    pub fn add_shared_library(&mut self, path: &str, dyn_syms: &[DynSymbol]) {
        self.add_shared_definitions(path, shared_definitions(dyn_syms));
    }

    /// Feeds a shared library given the names [`shared_definitions`] chose.
    pub fn add_shared_definitions<'a>(
        &mut self,
        path: &str,
        names: impl IntoIterator<Item = &'a str>,
    ) {
        let symbols = names.into_iter().map(|name| CommonSymbol {
            name,
            role: Some(CommonRole::Definition),
            restricted: false,
        });
        self.add_input(path, true, symbols);
    }

    /// The diagnostics in emission order.
    pub fn into_warnings(self) -> Vec<String> {
        self.warnings
    }
}

/// Classifies a relocatable object's symbol; `None` when it cannot matter.
pub fn object_symbol(
    name: &str,
    binding: u8,
    sym_type: u8,
    shndx: u16,
    size: u64,
    visibility: u8,
) -> Option<CommonSymbol<'_>> {
    if name.is_empty() || binding == STB_LOCAL || matches!(sym_type, STT_SECTION | STT_FILE) {
        return None;
    }
    let role = elf_common_role(binding, shndx, size);
    let restricted = visibility != STV_DEFAULT;
    (role.is_some() || restricted).then_some(CommonSymbol {
        name,
        role,
        restricted,
    })
}

/// Classifies an ELF symbol for [`CommonDiagnostics`]: `None` for locals,
/// weak symbols and undefined references.
pub fn elf_common_role(binding: u8, shndx: u16, size: u64) -> Option<CommonRole> {
    use crate::backend::elf::{SHN_COMMON, SHN_UNDEF, STB_GLOBAL};
    const STB_GNU_UNIQUE: u8 = 10;
    if binding != STB_GLOBAL && binding != STB_GNU_UNIQUE {
        return None;
    }
    match shndx {
        SHN_UNDEF => None,
        SHN_COMMON => Some(CommonRole::Common(size)),
        _ => Some(CommonRole::Definition),
    }
}

/// Whether a shared-library export with `st_info` = `info` takes part: a
/// strong data definition ([`super::shared_def_overrides_common`]).
pub fn dynamic_definition(info: u8) -> bool {
    super::symbols::shared_def_overrides_common(info)
}

/// The default-version exports of a library that take part.
pub fn shared_definitions(dyn_syms: &[DynSymbol]) -> impl Iterator<Item = &str> {
    dyn_syms
        .iter()
        .filter(|sym| sym.is_default_ver && dynamic_definition(sym.info))
        .map(|sym| sym.name.as_str())
}

/// Prints `warnings` and honors `--fatal-warnings` like bfd: every warning
/// is reported, then the link fails if any was emitted.
pub fn report_common_warnings(warnings: &[String], fatal: bool) -> Result<(), String> {
    for warning in warnings {
        eprintln!("lccc-ld: {warning}");
    }
    match warnings.first() {
        Some(first) if fatal => Err(format!("warning treated as error: {first}")),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use CommonRole::{Common, Definition};

    fn run(inputs: &[(&str, bool, &[(&str, CommonRole)])]) -> Vec<String> {
        let mut d = CommonDiagnostics::default();
        for (file, dynamic, syms) in inputs {
            let symbols = syms.iter().map(|&(name, role)| CommonSymbol {
                name,
                role: Some(role),
                restricted: false,
            });
            d.add_input(file, *dynamic, symbols);
        }
        d.into_warnings()
    }

    /// The single-conflict messages, measured with `ld --warn-common`.
    #[test]
    fn pairwise_messages_match_gnu_ld() {
        assert_eq!(
            run(&[
                ("c3.o", false, &[("q", Common(4))]),
                ("c4.o", false, &[("q", Definition)])
            ]),
            ["c4.o: warning: definition of `q' overriding common from c3.o"]
        );
        assert_eq!(
            run(&[
                ("c4.o", false, &[("q", Definition)]),
                ("c3.o", false, &[("q", Common(4))])
            ]),
            ["c3.o: warning: common of `q' overridden by definition from c4.o"]
        );
        assert_eq!(
            run(&[
                ("c1.o", false, &[("z", Common(4))]),
                ("c2.o", false, &[("z", Common(4))])
            ]),
            ["c2.o and c1.o: warning: multiple common of `z'"]
        );
    }

    /// `r1(4) r2(32) r3(4)`: the larger common takes ownership, so r3 is
    /// blamed on r2, not on the first registrant.
    #[test]
    fn larger_common_takes_ownership() {
        assert_eq!(
            run(&[
                ("r1.o", false, &[("r", Common(4))]),
                ("r2.o", false, &[("r", Common(32))]),
                ("r3.o", false, &[("r", Common(4))]),
            ]),
            [
                "r2.o: warning: common of `r' overriding smaller common from r1.o",
                "r3.o: warning: common of `r' overridden by larger common from r2.o",
            ]
        );
    }

    /// A shared-library definition owns the diagnostics once it has seen a
    /// common, a second library stays silent, and a regular definition
    /// replaces a library owner without a message.
    #[test]
    fn shared_library_ownership_matches_gnu_ld() {
        assert_eq!(
            run(&[
                ("s1.o", false, &[("s", Common(4))]),
                ("./libs.so", true, &[("s", Definition)]),
                ("s2.o", false, &[("s", Common(32))]),
                ("./libs2.so", true, &[("s", Definition)]),
            ]),
            [
                "./libs.so: warning: definition of `s' overriding common from s1.o",
                "s2.o: warning: common of `s' overridden by definition from ./libs.so",
            ]
        );
        assert_eq!(
            run(&[
                ("./libs.so", true, &[("s", Definition)]),
                ("s3.o", false, &[("s", Definition)]),
                ("s1.o", false, &[("s", Common(4))]),
            ]),
            ["s1.o: warning: common of `s' overridden by definition from s3.o"]
        );
    }

    /// Weak definitions, undefined references and locals never participate.
    #[test]
    fn non_participants_are_silent() {
        use crate::backend::elf::{SHN_COMMON, SHN_UNDEF, STB_GLOBAL, STB_LOCAL, STB_WEAK};
        assert_eq!(elf_common_role(STB_WEAK, 3, 4), None);
        assert_eq!(elf_common_role(STB_LOCAL, 3, 4), None);
        assert_eq!(elf_common_role(STB_GLOBAL, SHN_UNDEF, 0), None);
        assert_eq!(elf_common_role(STB_GLOBAL, SHN_COMMON, 8), Some(Common(8)));
        assert_eq!(elf_common_role(STB_GLOBAL, 3, 4), Some(Definition));
        // Library exports: only strong data definitions meet a common.
        use crate::backend::elf::{STT_FUNC, STT_GNU_IFUNC, STT_NOTYPE, STT_OBJECT};
        let info = |binding: u8, kind: u8| (binding << 4) | kind;
        assert!(dynamic_definition(info(STB_GLOBAL, STT_OBJECT)));
        assert!(dynamic_definition(info(STB_GLOBAL, STT_NOTYPE)));
        assert!(dynamic_definition(info(10, STT_OBJECT)));
        assert!(!dynamic_definition(info(STB_WEAK, STT_OBJECT)));
        assert!(!dynamic_definition(info(STB_GLOBAL, STT_FUNC)));
        assert!(!dynamic_definition(info(STB_GLOBAL, STT_GNU_IFUNC)));
        assert!(run(&[("r1.o", false, &[("r", Common(4))]), ("rw.o", false, &[])]).is_empty());
    }

    /// Non-default visibility in a regular object keeps library definitions
    /// out and drops a library owner silently (`ld --warn-common`, with
    /// cv.o = hidden common, hr.o = hidden reference, c3.o = common).
    #[test]
    fn restricted_visibility_matches_gnu_ld() {
        use crate::backend::elf::{SHN_COMMON, SHN_UNDEF, STB_GLOBAL, STT_OBJECT, STV_HIDDEN};
        let cv = object_symbol("s", STB_GLOBAL, STT_OBJECT, SHN_COMMON, 4, STV_HIDDEN);
        let hr = object_symbol("s", STB_GLOBAL, STT_OBJECT, SHN_UNDEF, 0, STV_HIDDEN);
        let c3 = object_symbol("s", STB_GLOBAL, STT_OBJECT, SHN_COMMON, 4, STV_DEFAULT);
        assert_eq!(hr.and_then(|sym| sym.role), None);
        let replay = |inputs: &[(&str, Option<CommonSymbol<'static>>)]| {
            let mut d = CommonDiagnostics::default();
            for (file, sym) in inputs {
                if file.ends_with(".so") {
                    d.add_shared_definitions(file, ["s"]);
                } else {
                    d.add_input(file, false, *sym);
                }
            }
            d.into_warnings()
        };
        let lib = ("./libl.so", None);
        let multiple = ["c3.o and cv.o: warning: multiple common of `s'"];
        assert_eq!(replay(&[lib, ("cv.o", cv), ("c3.o", c3)]), multiple);
        assert_eq!(replay(&[("cv.o", cv), lib, ("c3.o", c3)]), multiple);
        assert_eq!(
            replay(&[("c3.o", c3), lib, ("cv.o", cv)]),
            ["./libl.so: warning: definition of `s' overriding common from c3.o"]
        );
        assert!(replay(&[("hr.o", hr), ("c3.o", c3), lib]).is_empty());
    }

    /// A library fed again by a group rescan is not a new input.
    #[test]
    fn rescanned_library_is_ignored() {
        let mut d = CommonDiagnostics::default();
        let c = [CommonSymbol {
            name: "s",
            role: Some(Common(4)),
            restricted: false,
        }];
        d.add_input("s1.o", false, c);
        d.add_shared_definitions("./libs.so", ["s"]);
        d.add_shared_definitions("./libs.so", ["s"]);
        d.add_input("s2.o", false, c);
        assert_eq!(
            d.into_warnings(),
            [
                "./libs.so: warning: definition of `s' overriding common from s1.o",
                "s2.o: warning: common of `s' overridden by definition from ./libs.so",
            ]
        );
    }
}
