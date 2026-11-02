//! Phase 1: Input file loading, archive resolution, and shared library symbol discovery.
//!
//! Handles reading ELF objects, static archives (both regular and thin), and shared
//! libraries. Resolves undefined symbols by demand-loading archive members, with
//! group iteration to handle circular dependencies between libraries.

use super::elf_read::*;
use super::relocations::resolve_archive_members;
use crate::backend::linker_common;
use crate::backend::linker_common::warn_common::CommonDiagnostics;
use crate::common::fx_hash::{FxHashMap, FxHashSet};

/// Exports of the linked shared libraries, bound with GNU ld's policy (the
/// one `linker_common::register_shared_symbols` applies for x86/AArch64):
/// the first library in link order that exports a name provides it, and an
/// unversioned reference binds only to a default (`@@`) version — a hidden
/// `foo@V1` beside `foo@@V2` must not win because it comes later in the
/// table.  The provider is recorded so imports get their `Verneed`.
///
/// Libraries are read after the command line's objects, so a common of
/// those meets them "common first": the first linked library with a strong
/// data definition takes it (`shared_def_overrides_common`), even after
/// another library's weak one -- that export is the one bound.
#[derive(Default)]
pub struct SharedLibSyms {
    syms: FxHashMap<String, DynSymbol>,
    provider: FxHashMap<String, String>,
    /// Commons of the objects read before the libraries, minus names some
    /// object gives non-default visibility (`expect_commons`).
    commons: FxHashSet<String>,
    /// The members of `commons` a library definition took.
    taken: FxHashSet<String>,
    /// `--warn-common`: each library read, in order, with its strong
    /// default-version definitions; `None` unless requested.
    libraries: Option<Vec<(String, Vec<String>)>>,
}

impl SharedLibSyms {
    /// Remember every library read from now on for `--warn-common`.
    pub fn record_libraries(&mut self) {
        self.libraries = Some(Vec::new());
    }

    /// A library at `path` was read (a no-op unless recording).
    pub fn note_library(&mut self, path: &str, syms: &[DynSymbol]) {
        use crate::backend::linker_common::warn_common::shared_definitions;
        if let Some(libraries) = &mut self.libraries {
            let defs = shared_definitions(syms).map(str::to_string).collect();
            libraries.push((path.to_string(), defs));
        }
    }

    /// Feed the recorded libraries, in reading order.
    pub fn replay_libraries(&self, common: &mut CommonDiagnostics) {
        for (path, defs) in self.libraries.iter().flatten() {
            common.add_shared_definitions(path, defs.iter().map(String::as_str));
        }
    }

    /// Record the commons of the objects read so far, before any library.
    pub fn expect_commons(&mut self, input_objs: &[(String, ElfObject)]) {
        let symbols = || input_objs.iter().flat_map(|(_, obj)| &obj.symbols);
        let restricted: FxHashSet<&str> = symbols()
            .filter(|sym| sym.binding() != STB_LOCAL && sym.visibility() != STV_DEFAULT)
            .map(|sym| sym.name.as_str())
            .collect();
        self.commons = symbols()
            .filter(|sym| sym.shndx == SHN_COMMON && !restricted.contains(sym.name.as_str()))
            .map(|sym| sym.name.to_string())
            .collect();
    }

    /// Whether `sym` takes over one of the expected commons.
    pub fn takes_common(&self, sym: &DynSymbol) -> bool {
        sym.is_default_ver
            && self.commons.contains(&sym.name)
            && !self.taken.contains(&sym.name)
            && linker_common::shared_def_overrides_common(sym.info)
    }

    /// Whether a library definition took the common `name` read before the
    /// libraries (`Some(false)`: it stays a common), or `None` when no such
    /// common was expected.
    pub fn common_taken(&self, name: &str) -> Option<bool> {
        self.commons
            .contains(name)
            .then(|| self.taken.contains(name))
    }

    /// Offer one export of `soname` from a library in the link; the first
    /// default-version export wins, except over an expected common.
    pub fn offer(&mut self, sym: DynSymbol, soname: &str) {
        if self.takes_common(&sym) {
            self.taken.insert(sym.name.clone());
        } else if !sym.is_default_ver || self.syms.contains_key(&sym.name) {
            return;
        }
        self.provider.insert(sym.name.clone(), soname.to_string());
        self.syms.insert(sym.name.clone(), sym);
    }

    /// Bring the library `path` (`soname`, exporting `syms`) into the link
    /// unless it is `as_needed` and, read now, would neither satisfy a
    /// reference (`satisfies`) no library in the link already answers, nor
    /// take a common -- one of `commons_before` when given, the commons read
    /// before it -- and is not in it already: bfd unloads such a library,
    /// symbols included.  Returns whether it is in the link.
    pub fn link_library(
        &mut self,
        path: &str,
        soname: &str,
        syms: Vec<DynSymbol>,
        as_needed: bool,
        satisfies: impl Fn(&str) -> bool,
        commons_before: Option<&FxHashSet<&str>>,
        needed: &mut Vec<String>,
    ) -> bool {
        let wanted = |si: &DynSymbol| {
            (satisfies(&si.name) && !self.syms.contains_key(&si.name))
                || (commons_before.is_none_or(|before| before.contains(si.name.as_str()))
                    && self.takes_common(si))
        };
        let soname_linked = needed.iter().any(|n| n == soname);
        if as_needed && !soname_linked && !syms.iter().any(wanted) {
            return false;
        }
        self.note_library(path, &syms);
        for si in syms {
            self.offer(si, soname);
        }
        if !soname_linked {
            needed.push(soname.to_string());
        }
        true
    }

    /// The shared library that provides `name`.
    pub fn provider_of(&self, name: &str) -> Option<&str> {
        self.provider.get(name).map(String::as_str)
    }

    /// `(provider soname, version)` of an import bound to a versioned export.
    pub fn import_version(&self, name: &str) -> Option<(&str, &str)> {
        Some((
            self.provider.get(name)?.as_str(),
            self.syms.get(name)?.version.as_deref()?,
        ))
    }
}

impl std::ops::Deref for SharedLibSyms {
    type Target = FxHashMap<String, DynSymbol>;
    fn deref(&self) -> &Self::Target {
        &self.syms
    }
}

/// Load a single input file (ELF object, archive, or thin archive) into
/// `input_objs`. Archives are saved to `inline_archive_paths` for later
/// demand-driven extraction; ELF shared objects named positionally on the
/// command line go to `positional_shared_libs` for DT_NEEDED and
/// dynamic-symbol registration (bfd semantics) rather than being parsed as
/// relocatable objects (that used to fail with a bogus
/// "not a relocatable object (type=3)" diagnostic).
pub fn load_input_files(
    all_inputs: &[String],
    input_objs: &mut Vec<(String, ElfObject)>,
    inline_archive_paths: &mut Vec<String>,
    positional_shared_libs: &mut Vec<(String, usize)>,
) -> Result<(), String> {
    for path in all_inputs {
        if !std::path::Path::new(path).exists() {
            return Err(format!("linker input file not found: {}", path));
        }
        let data = std::fs::read(path).map_err(|e| format!("Cannot read {}: {}", path, e))?;

        if (data.len() >= 8 && &data[0..8] == b"!<arch>\n") || is_thin_archive(&data) {
            inline_archive_paths.push(path.clone());
        } else if data.len() >= 18 && &data[0..4] == b"\x7fELF" {
            let e_type = u16::from_le_bytes([data[16], data[17]]);
            if e_type == ET_DYN {
                positional_shared_libs.push((path.clone(), input_objs.len()));
            } else {
                let obj = parse_object(&data, path).map_err(|e| format!("{}: {}", path, e))?;
                input_objs.push((path.clone(), obj));
            }
        }
        // Skip non-ELF/non-archive files (e.g. linker scripts)
    }
    Ok(())
}

/// Register the dynamic symbols of positionally-named shared libraries and
/// record their DT_NEEDED entries (DT_SONAME, else the path as named:
/// `linker_common::unsonamed_needed_name`).  Each comes with the number of
/// `input_objs` named before it: an `--as-needed` one is kept, as by GNU
/// ld, only for a non-weak reference or a common those objects hold.
pub fn register_positional_shared_libs(
    positional_shared_libs: &[(String, usize)],
    as_needed: impl Fn(&str) -> bool,
    input_objs: &[(String, ElfObject)],
    shared_lib_syms: &mut SharedLibSyms,
    actual_needed_libs: &mut Vec<String>,
) {
    // What the objects before the library define, reference non-weakly
    // and hold as commons -- grown one object at a time.
    let mut defined: FxHashSet<&str> = FxHashSet::default();
    let mut undefined: FxHashSet<&str> = FxHashSet::default();
    let mut commons: FxHashSet<&str> = FxHashSet::default();
    let mut read = 0;
    for (path, objs_before) in positional_shared_libs {
        for (_, obj) in input_objs.get(read..*objs_before).unwrap_or(&[]) {
            for sym in obj.symbols.iter().filter(|s| s.binding() != STB_LOCAL) {
                let name = sym.name.as_str();
                match sym.shndx {
                    SHN_UNDEF
                        if sym.binding() != STB_WEAK
                            && !defined.contains(name)
                            && !commons.contains(name) =>
                    {
                        undefined.insert(name);
                    }
                    SHN_UNDEF => {}
                    // A common defines the name for the references too.
                    SHN_COMMON => {
                        commons.insert(name);
                        undefined.remove(name);
                    }
                    _ => {
                        defined.insert(name);
                        undefined.remove(name);
                    }
                }
            }
        }
        read = read.max(*objs_before);
        let Ok(data) = std::fs::read(path) else {
            continue;
        };
        let needed = linker_common::parse_soname(&data)
            .unwrap_or_else(|| linker_common::unsonamed_needed_name(path, false));
        shared_lib_syms.link_library(
            path,
            &needed,
            linker_common::parse_shared_library_symbols(&data, path).unwrap_or_default(),
            as_needed(path),
            |name| undefined.contains(name),
            Some(&commons),
            actual_needed_libs,
        );
    }
}

/// The `--as-needed` state of each of `needed_libs`: a library the user
/// named (before `implicit_from`) has the state of its command-line
/// position; of the driver's, gcc links its shared unwinder as-needed
/// (`-lgcc --as-needed -lgcc_s`), and the default `-lm` -- not in gcc's spec
/// at all -- is as-needed too, so it costs a program that uses no libm
/// function no library load.
pub fn libs_as_needed(
    needed_libs: &[String],
    implicit_from: usize,
    inputs: &[linker_common::InputItem],
) -> Vec<bool> {
    needed_libs
        .iter()
        .enumerate()
        .map(|(i, lib)| {
            if i < implicit_from {
                linker_common::input_as_needed(inputs, lib)
            } else {
                lib == "gcc_s" || lib == "m"
            }
        })
        .collect()
}

/// Scan input objects to build initial defined/undefined symbol sets.
///
/// `undefined_syms` holds the names referenced non-weakly and not defined:
/// GNU ld extracts an archive member, or keeps an `--as-needed` library, only
/// for such a reference (`ref_regular_nonweak`).  A weak one binds to a
/// definition the link has anyway -- linked libraries' exports are imported
/// from `shared_lib_syms`, not from this set.
pub fn collect_initial_symbols(
    input_objs: &[(String, ElfObject)],
    defined_syms: &mut FxHashSet<String>,
    undefined_syms: &mut FxHashSet<String>,
) {
    for (_, obj) in input_objs {
        for sym in &obj.symbols {
            if sym.shndx != SHN_UNDEF && sym.binding() != STB_LOCAL && !sym.name.is_empty() {
                defined_syms.insert(sym.name.to_string());
            }
        }
    }
    for (_, obj) in input_objs {
        for sym in &obj.symbols {
            if sym.shndx == SHN_UNDEF
                && !sym.name.is_empty()
                && !matches!(sym.binding(), STB_LOCAL | STB_WEAK)
                && !defined_syms.contains(sym.name.as_str())
            {
                undefined_syms.insert(sym.name.to_string());
            }
        }
    }
}

/// Discover symbols exported by shared libraries referenced via `-l` flags.
///
/// For each needed library, tries to find `.so` files in the search paths and
/// reads their dynamic symbol tables. Handles GNU linker scripts that reference
/// other `.so` files or `-l` flags. Returns the actual NEEDED soname strings.
///
/// Also loads archive members from `_nonshared.a` files referenced in linker scripts.
pub fn discover_shared_lib_symbols(
    needed_libs: &[String],
    lib_as_needed: &[bool],
    lib_search_paths: &[String],
    input_objs: &mut Vec<(String, ElfObject)>,
    defined_syms: &mut FxHashSet<String>,
    undefined_syms: &mut FxHashSet<String>,
    shared_lib_syms: &mut SharedLibSyms,
    actual_needed_libs: &mut Vec<String>,
) {
    use super::relocations::find_versioned_soname;

    for (i, libname) in needed_libs.iter().enumerate() {
        let so_name = if let Some(exact) = libname.strip_prefix(':') {
            exact.to_string()
        } else {
            format!("lib{}.so", libname)
        };
        for dir in lib_search_paths {
            let path = format!("{}/{}", dir, so_name);
            if !std::path::Path::new(&path).exists() {
                continue;
            }
            let data = match std::fs::read(&path) {
                Ok(d) => d,
                Err(_) => continue,
            };
            let as_needed = lib_as_needed[i];
            if data.starts_with(b"/* GNU ld script")
                || data.starts_with(b"OUTPUT_FORMAT")
                || data.starts_with(b"GROUP")
                || data.starts_with(b"INPUT")
            {
                let before = actual_needed_libs.len();
                process_linker_script(
                    &data,
                    dir,
                    lib_search_paths,
                    as_needed,
                    input_objs,
                    defined_syms,
                    undefined_syms,
                    shared_lib_syms,
                    actual_needed_libs,
                );
                // A script naming no loadable shared object: fall back to the
                // versioned runtime object next to it (Debian's lib*.so).
                if actual_needed_libs.len() == before
                    && !as_needed
                    && let Some(soname) = find_versioned_soname(dir, libname)
                    && !actual_needed_libs.contains(&soname)
                {
                    actual_needed_libs.push(soname);
                }
            } else if data.len() >= 4 && &data[0..4] == b"\x7fELF" {
                // DT_NEEDED for a real ELF .so: its DT_SONAME, else the file
                // name we linked against — exactly bfd's behaviour.  (Pre-fix
                // only the versioned-soname glob above added NEEDED entries,
                // so `-lfoo` against an unversioned libfoo.so resolved its
                // symbols at link time and then failed to find them at
                // runtime — observed on real riscv64 glibc under qemu.)
                let needed = linker_common::parse_soname(&data).unwrap_or_else(|| so_name.clone());
                shared_lib_syms.link_library(
                    &path,
                    &needed,
                    linker_common::parse_shared_library_symbols(&data, &path).unwrap_or_default(),
                    as_needed,
                    |name| undefined_syms.contains(name) && !defined_syms.contains(name),
                    None,
                    actual_needed_libs,
                );
            }
            break;
        }
    }
}

/// Process a GNU linker script, extracting `.so` references and `-l` library flags.
///
/// Every shared object the script names is a link input like `-l` itself, so
/// it gets a DT_NEEDED -- its DT_SONAME, else the file name, as bfd records
/// it -- unless it is linked as-needed (inside `AS_NEEDED ( ... )`, or the
/// whole script when `as_needed`) and defines none of the inputs' undefined
/// symbols. (Pre-fix only a `lib<name>.so.*` next to the script became
/// NEEDED: right for glibc's libc.so, wrong for gcc's libgcc_s.so, which
/// lives in the compiler directory while libgcc_s.so.1 is in the sysroot --
/// `_Unwind_*` bound at link time and failed to load at run time.)
fn process_linker_script(
    data: &[u8],
    dir: &str,
    lib_search_paths: &[String],
    as_needed: bool,
    input_objs: &mut Vec<(String, ElfObject)>,
    defined_syms: &mut FxHashSet<String>,
    undefined_syms: &mut FxHashSet<String>,
    shared_lib_syms: &mut SharedLibSyms,
    actual_needed_libs: &mut Vec<String>,
) {
    let text = String::from_utf8_lossy(data)
        .replace('(', " ( ")
        .replace(')', " ) ");
    // Open groups: whether each one is `AS_NEEDED (`.
    let mut groups: Vec<bool> = Vec::new();
    let mut after_as_needed = false;
    for token in text.split_whitespace() {
        match token {
            "(" => {
                groups.push(after_as_needed);
                after_as_needed = false;
                continue;
            }
            ")" => {
                groups.pop();
                continue;
            }
            _ => after_as_needed = token == "AS_NEEDED",
        }
        let token = token.trim_matches(',');
        let token_as_needed = as_needed || groups.iter().any(|&g| g);
        // Every GROUP/INPUT member is a link input resolved like bfd does --
        // `-lNAME` by the `-l` search (shared object first), a path as
        // written, else next to the script, else in the search path -- and
        // then handled by what the FILE is, not by its name: a shared object
        // supplies symbols (and DT_NEEDED unless as-needed and unused), an
        // archive supplies members. (Pre-fix only names containing ".so"
        // were considered: glibc's `libc_nonshared.a` was never read, so
        // `atexit` came from static libc.a; a script's `-lNAME` never got
        // a DT_NEEDED.)
        let Some(path) = resolve_script_input(token, dir, lib_search_paths) else {
            continue;
        };
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        if bytes.starts_with(b"!<arch>\n") || is_thin_archive(&bytes) {
            load_nonshared_archive(&path, input_objs, defined_syms, undefined_syms);
        } else if bytes.starts_with(b"\x7fELF") {
            let Ok(syms) = linker_common::parse_shared_library_symbols(&bytes, &path) else {
                continue;
            };
            let soname = linker_common::parse_soname(&bytes).unwrap_or_else(|| {
                linker_common::unsonamed_needed_name(&path, token.starts_with("-l"))
            });
            shared_lib_syms.link_library(
                &path,
                &soname,
                syms,
                token_as_needed,
                |name| undefined_syms.contains(name) && !defined_syms.contains(name),
                None,
                actual_needed_libs,
            );
        }
    }
}

/// The file a linker-script token names: `-lNAME` by the dynamic `-l`
/// search; a file name by GNU's script lookup (`resolve_script_path`:
/// sysroot prefix for absolute names, then next to the script, the working
/// directory, the search path). `None` for keywords (`GROUP`,
/// `OUTPUT_FORMAT`, ...), their arguments, and files that do not exist.
fn resolve_script_input(token: &str, dir: &str, lib_search_paths: &[String]) -> Option<String> {
    if let Some(lib_name) = token.strip_prefix("-l") {
        return (!lib_name.is_empty())
            .then(|| linker_common::resolve_lib(lib_name, lib_search_paths, false))
            .flatten();
    }
    if !(token.contains(".so") || token.ends_with(".a")) {
        return None;
    }
    linker_common::resolve_script_path(token, Some(dir), lib_search_paths)
}

/// Load the members of an archive named by a linker script that resolve
/// currently undefined symbols (e.g. glibc's `libc_nonshared.a`).
fn load_nonshared_archive(
    path: &str,
    input_objs: &mut Vec<(String, ElfObject)>,
    defined_syms: &mut FxHashSet<String>,
    undefined_syms: &mut FxHashSet<String>,
) {
    if let Ok(archive_data) = std::fs::read(path) {
        if archive_data.len() >= 8 && &archive_data[0..8] == b"!<arch>\n" {
            if let Ok(members) = parse_archive(&archive_data) {
                resolve_archive_members(members, input_objs, defined_syms, undefined_syms);
            }
        } else if is_thin_archive(&archive_data) {
            if let Ok(members) = parse_thin_archive(&archive_data, path) {
                resolve_archive_members(members, input_objs, defined_syms, undefined_syms);
            }
        }
    }
}

/// Resolve undefined symbols from `.a` archives using group iteration.
///
/// Iterates over all archives repeatedly until no new symbols are resolved,
/// handling circular dependencies (e.g., libm -> libgcc -> libc -> libgcc).
/// The archive `-lNAME` links (`archive_name` is `libNAME.a`, or the file
/// of `-l:FILE`), by GNU's search: per directory, `libNAME.so` (a linker
/// script counts) before `libNAME.a` unless the link is static, and the
/// first hit ends the search -- a shared hit means no archive. (Pre-fix
/// `libc.a` was searched for every dynamic link: a symbol libc.so.6 does not
/// export by default pulled static glibc members -- and with them `_start`
/// from dl-support.o -- into dynamic outputs.)
fn archive_for_lib(
    libname: &str,
    archive_name: &str,
    lib_search_paths: &[String],
    is_static: bool,
) -> Option<String> {
    let shared_name = (!is_static && !libname.starts_with(':')).then(|| format!("lib{libname}.so"));
    for dir in lib_search_paths {
        if shared_name
            .as_ref()
            .is_some_and(|so| std::path::Path::new(&format!("{dir}/{so}")).exists())
        {
            return None;
        }
        let path = format!("{dir}/{archive_name}");
        if std::path::Path::new(&path).exists() {
            return Some(path);
        }
    }
    None
}

pub fn resolve_archives(
    inline_archive_paths: &[String],
    needed_libs: &[String],
    is_static: bool,
    lib_search_paths: &[String],
    input_objs: &mut Vec<(String, ElfObject)>,
    defined_syms: &mut FxHashSet<String>,
    undefined_syms: &mut FxHashSet<String>,
    shared_lib_syms: &FxHashMap<String, DynSymbol>,
) {
    let mut archive_paths: Vec<String> = Vec::new();
    let mut seen: FxHashSet<String> = FxHashSet::default();

    // Add inline archives first
    for path in inline_archive_paths {
        if !seen.contains(path) {
            seen.insert(path.clone());
            archive_paths.push(path.clone());
        }
    }
    // Then -l library archives
    for libname in needed_libs {
        let archive_name = if let Some(exact) = libname.strip_prefix(':') {
            exact.to_string()
        } else {
            format!("lib{}.a", libname)
        };
        if let Some(path) = archive_for_lib(libname, &archive_name, lib_search_paths, is_static)
            && seen.insert(path.clone())
        {
            archive_paths.push(path);
        }
    }

    // Group iteration until stable
    let mut group_changed = true;
    while group_changed {
        group_changed = false;
        let prev_count = input_objs.len();

        // Remove symbols available from shared libraries from undefined_syms.
        // This prevents archive members from being extracted just because they
        // define symbols already available via dynamic linking (e.g., stdio.o
        // from libc.a defining stderr/stdout when libc.so provides them).
        // Must be done each iteration since newly-extracted members may add
        // shared-lib-provided symbols back into undefined_syms.
        for sym_name in shared_lib_syms.keys() {
            if !defined_syms.contains(sym_name) {
                undefined_syms.remove(sym_name);
            }
        }

        for path in &archive_paths {
            let data = match std::fs::read(path) {
                Ok(d) => d,
                Err(_) => continue,
            };
            if data.len() >= 8 && &data[0..8] == b"!<arch>\n" {
                if let Ok(members) = parse_archive(&data) {
                    resolve_archive_members(members, input_objs, defined_syms, undefined_syms);
                }
            } else if is_thin_archive(&data) {
                if let Ok(members) = parse_thin_archive(&data, path) {
                    resolve_archive_members(members, input_objs, defined_syms, undefined_syms);
                }
            }
        }
        if input_objs.len() != prev_count {
            group_changed = true;
        }
    }
}

#[cfg(test)]
mod lib_search_tests {
    use super::{archive_for_lib, resolve_script_input};

    fn scratch(tag: &str) -> std::path::PathBuf {
        let d =
            std::env::temp_dir().join(format!("lccc_rv_libsearch_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn dynamic_l_search_never_takes_the_archive_behind_a_shared_object() {
        let (a, b) = (scratch("a"), scratch("b"));
        std::fs::write(a.join("libc.so"), "GROUP ( libc.so.6 )").unwrap();
        std::fs::write(a.join("libc.a"), "!<arch>\n").unwrap();
        std::fs::write(b.join("libgcc.a"), "!<arch>\n").unwrap();
        std::fs::write(b.join("libc.a"), "!<arch>\n").unwrap();
        let dirs = [a.display().to_string(), b.display().to_string()];
        // libc.so in the first directory ends the search: no libc.a at all.
        assert_eq!(archive_for_lib("c", "libc.a", &dirs, false), None);
        // A static link takes the first libc.a.
        assert_eq!(
            archive_for_lib("c", "libc.a", &dirs, true),
            Some(format!("{}/libc.a", dirs[0]))
        );
        // No shared object anywhere: the archive, from the later directory.
        assert_eq!(
            archive_for_lib("gcc", "libgcc.a", &dirs, false),
            Some(format!("{}/libgcc.a", dirs[1]))
        );
        // `-l:FILE` names the file exactly.
        assert_eq!(
            archive_for_lib(":libc.a", "libc.a", &dirs, false),
            Some(format!("{}/libc.a", dirs[0]))
        );
        let _ = (std::fs::remove_dir_all(&a), std::fs::remove_dir_all(&b));
    }

    #[test]
    fn script_members_resolve_by_path_script_dir_search_path_and_l_search() {
        let (script_dir, search) = (scratch("s"), scratch("p"));
        std::fs::write(script_dir.join("libc_nonshared.a"), "!<arch>\n").unwrap();
        std::fs::write(search.join("libgcc_s.so.1"), "").unwrap();
        std::fs::write(search.join("libgcc.a"), "!<arch>\n").unwrap();
        let dir = script_dir.display().to_string();
        let dirs = [search.display().to_string()];
        let abs = format!("{dir}/libc_nonshared.a");
        assert_eq!(
            resolve_script_input(&abs, "/nonexistent", &dirs),
            Some(abs.clone())
        );
        assert_eq!(
            resolve_script_input("libc_nonshared.a", &dir, &dirs),
            Some(abs)
        );
        assert_eq!(
            resolve_script_input("libgcc_s.so.1", &dir, &dirs),
            Some(format!("{}/libgcc_s.so.1", dirs[0]))
        );
        assert_eq!(
            resolve_script_input("-lgcc", &dir, &dirs),
            Some(format!("{}/libgcc.a", dirs[0]))
        );
        for keyword in [
            "GROUP",
            "AS_NEEDED",
            "elf64-littleriscv",
            "-l",
            "libmissing.so.1",
        ] {
            assert_eq!(
                resolve_script_input(keyword, &dir, &dirs),
                None,
                "{keyword}"
            );
        }
        let _ = (
            std::fs::remove_dir_all(&script_dir),
            std::fs::remove_dir_all(&search),
        );
    }
}
