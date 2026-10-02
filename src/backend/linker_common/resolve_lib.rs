//! Library name resolution helper.
//!
//! Resolves `-l` library names to filesystem paths by searching library
//! directories, handling both exact (`:filename`) and prefix (`libfoo.so/.a`)
//! forms.

use std::path::Path;

/// Resolve a library name to a path by searching directories.
///
/// Handles both `-l:filename` (exact match) and `-lfoo` (lib prefix search).
/// When `prefer_static` is true, searches for `.a` before `.so`.
pub fn resolve_lib(name: &str, paths: &[String], prefer_static: bool) -> Option<String> {
    if let Some(exact) = name.strip_prefix(':') {
        for dir in paths {
            let p = format!("{}/{}", dir, exact);
            if Path::new(&p).exists() {
                return Some(p);
            }
        }
        return None;
    }
    if prefer_static {
        for dir in paths {
            let a = format!("{}/lib{}.a", dir, name);
            if Path::new(&a).exists() {
                return Some(a);
            }
            let so = format!("{}/lib{}.so", dir, name);
            if Path::new(&so).exists() {
                return Some(so);
            }
        }
    } else {
        for dir in paths {
            let so = format!("{}/lib{}.so", dir, name);
            if Path::new(&so).exists() {
                return Some(so);
            }
            let a = format!("{}/lib{}.a", dir, name);
            if Path::new(&a).exists() {
                return Some(a);
            }
        }
    }
    None
}

/// Resolve `-lNAME` the way GNU ld does at one position of the command line.
///
/// Each directory is tried in order; within a directory `libNAME.so` wins
/// over `libNAME.a` unless `static_only` (`-Bstatic`/`-static` in effect at
/// this `-l`), in which case only `libNAME.a` qualifies — a directory holding
/// just the shared object is skipped, never silently used.  `-l:FILE` names
/// an exact file in either mode.
pub fn resolve_lib_positional(name: &str, paths: &[String], static_only: bool) -> Option<String> {
    if name.starts_with(':') {
        return resolve_lib(name, paths, static_only);
    }
    paths.iter().find_map(|dir| {
        let so = format!("{dir}/lib{name}.so");
        if !static_only && Path::new(&so).exists() {
            return Some(so);
        }
        let a = format!("{dir}/lib{name}.a");
        Path::new(&a).exists().then_some(a)
    })
}

/// Resolve a file named inside a linker script (`GROUP` / `INPUT`).
///
/// GNU ld semantics: an absolute name is looked up under `LCCC_SYSROOT`
/// first (a script inside the sysroot has its absolute names prefixed), then
/// exactly as written; a relative name is tried next to the script, then
/// against the working directory, then in the library search path.
///
/// `is_file`, not `exists`: a *directory* carrying the entry's name is not an
/// input, and `open()` on one succeeds on Linux — the caller would map it and
/// report a bogus ELF parse error instead of `cannot find …`.
pub fn resolve_script_path(
    named: &str,
    script_dir: Option<&str>,
    lib_paths: &[String],
) -> Option<String> {
    if named.starts_with('/') {
        let prefixed = crate::backend::common::with_sysroot_prefix(named);
        if prefixed != named && Path::new(&prefixed).is_file() {
            return Some(prefixed);
        }
        return Path::new(named).is_file().then(|| named.to_string());
    }
    script_dir
        .map(|dir| format!("{dir}/{named}"))
        .into_iter()
        .chain(std::iter::once(named.to_string()))
        .chain(lib_paths.iter().map(|dir| format!("{dir}/{named}")))
        .find(|candidate| Path::new(candidate).is_file())
}

/// How GNU ld spells an unresolved linker-script input in its diagnostic:
/// the path as written, or `-lNAME` for a library reference.
pub fn script_input_spelling(input: &crate::backend::elf::LinkerScriptInput) -> String {
    use crate::backend::elf::LinkerScriptEntry;
    match &input.entry {
        LinkerScriptEntry::Path(p) => p.clone(),
        LinkerScriptEntry::Lib(name) => format!("-l{name}"),
    }
}

/// The input list of a relocatable (`-r`) or script-driven (`-T`) link:
/// the positional `files` with every `-lNAME` of the command line expanded,
/// in place, into the file it names.
///
/// `ordered_args` is the argument vector with the positional files at their
/// command-line positions; it is read with the same positional-state parser
/// as ordinary links, so `--whole-archive`, `-Bstatic`/`-Bdynamic`/`-static`
/// and `--push-state`/`--pop-state` mean exactly what they mean there.  `-L`
/// directories are searched first, then `extra_dirs` (a script's
/// `SEARCH_DIR`s, which GNU ld searches after `-L`).
///
/// Only archives and relocatable objects can be merged by these emitters: a
/// library that resolves to a shared object (or a text `.so` linker script)
/// is an error naming the file, never a silent drop -- before this, `-l` was
/// ignored outright in both modes and surfaced, at best, as undefined
/// symbols.  A positional file keeps the `--whole-archive` flag the caller
/// recorded for it; a non-library item of the parse that is not the next
/// expected file (the value of an option the parser does not know takes one
/// that happens to name an existing file) is skipped, and files the parse did
/// not see (non-existent paths) are appended so the loader reports them.
pub fn expand_file_mode_libs(
    ordered_args: &[String],
    files: &[(String, bool)],
    extra_dirs: &[String],
    mode: &str,
) -> Result<Vec<(String, bool)>, String> {
    let parsed = super::parse_linker_args(ordered_args);
    let mut dirs = parsed.extra_lib_paths.clone();
    dirs.extend(extra_dirs.iter().cloned());
    let mut out = Vec::with_capacity(files.len());
    let mut next_file = 0;
    for item in &parsed.inputs {
        if !item.is_lib {
            if files.get(next_file).is_some_and(|(f, _)| *f == item.name) {
                out.push(files[next_file].clone());
                next_file += 1;
            }
            continue;
        }
        let path = resolve_lib_positional(&item.name, &dirs, item.static_search)
            .ok_or_else(|| format!("cannot find -l{}", item.name))?;
        let mut head = [0u8; 18];
        let n = std::fs::File::open(&path)
            .and_then(|mut f| std::io::Read::read(&mut f, &mut head))
            .map_err(|e| format!("-l{}: cannot read {path}: {e}", item.name))?;
        let archive = n >= 8 && (&head[..8] == b"!<arch>\n" || &head[..8] == b"!<thin>\n");
        let elf_rel = n >= 18
            && head[..4] == *b"\x7fELF"
            && match head[5] {
                1 => u16::from_le_bytes([head[16], head[17]]) == 1,
                2 => u16::from_be_bytes([head[16], head[17]]) == 1,
                _ => false,
            };
        if !archive && !elf_rel {
            return Err(format!(
                "-l{}: {path} is not an archive or relocatable object; {mode} links \
                 merge static inputs only (use -Bstatic or name the archive)",
                item.name
            ));
        }
        out.push((path, item.whole_archive));
    }
    out.extend(files[next_file..].iter().cloned());
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::{
        expand_file_mode_libs, resolve_lib_positional, resolve_script_path, script_input_spelling,
    };
    use crate::backend::elf::{LinkerScriptEntry, LinkerScriptInput};

    fn temp_root(tag: &str) -> std::path::PathBuf {
        let root =
            std::env::temp_dir().join(format!("lccc_script_path_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn script_path_prefers_script_dir_then_search_path() {
        let root = temp_root("order");
        let script_dir = root.join("scriptdir");
        let lib_dir = root.join("libdir");
        std::fs::create_dir_all(&script_dir).unwrap();
        std::fs::create_dir_all(&lib_dir).unwrap();
        std::fs::write(lib_dir.join("libm.so"), b"").unwrap();
        let libs = vec![lib_dir.display().to_string()];
        let script_dir_string = script_dir.display().to_string();
        let dir = Some(script_dir_string.as_str());

        // Only in the library search path.
        assert_eq!(
            resolve_script_path("libm.so", dir, &libs),
            Some(format!("{}/libm.so", lib_dir.display()))
        );
        // Next to the script wins over the search path (GNU ld order).
        std::fs::write(script_dir.join("libm.so"), b"").unwrap();
        assert_eq!(
            resolve_script_path("libm.so", dir, &libs),
            Some(format!("{}/libm.so", script_dir.display()))
        );
        // Nothing anywhere: None, so the caller reports the operand by name.
        assert_eq!(resolve_script_path("libnope.so", dir, &libs), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn script_path_rejects_directory_and_missing_absolute_names() {
        let root = temp_root("kinds");
        // A directory carrying the entry's name is not an input: `exists()`
        // said yes and the loader then mapped a directory.
        std::fs::create_dir_all(root.join("libdir.so")).unwrap();
        let libs: Vec<String> = Vec::new();
        assert_eq!(
            resolve_script_path(
                "libdir.so",
                Some(root.display().to_string().as_str()),
                &libs
            ),
            None,
            "a directory must not resolve as a linker-script input"
        );
        let missing = format!("{}/definitely-absent.so", root.display());
        assert_eq!(resolve_script_path(&missing, None, &libs), None);
        let file = root.join("present.so");
        std::fs::write(&file, b"").unwrap();
        assert_eq!(
            resolve_script_path(&file.display().to_string(), None, &libs),
            Some(file.display().to_string())
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn script_path_prefixes_absolute_names_with_the_sysroot() {
        use crate::test_support::EnvGuard;
        let root = temp_root("sysroot");
        let rootfs = root.join("rootfs");
        std::fs::create_dir_all(rootfs.join("usr/lib")).unwrap();
        std::fs::write(rootfs.join("usr/lib/libc.so.6"), b"").unwrap();
        let libs: Vec<String> = Vec::new();
        let _guard = EnvGuard::set("LCCC_SYSROOT", &rootfs.display().to_string());
        assert_eq!(
            resolve_script_path("/usr/lib/libc.so.6", None, &libs),
            Some(format!("{}/usr/lib/libc.so.6", rootfs.display())),
            "an absolute script name resolves under the sysroot first"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn script_input_spelling_matches_the_gnu_ld_operand() {
        let path = LinkerScriptInput {
            entry: LinkerScriptEntry::Path("/lib/libc.so.6".into()),
            as_needed: false,
        };
        let lib = LinkerScriptInput {
            entry: LinkerScriptEntry::Lib("gcc_s".into()),
            as_needed: true,
        };
        assert_eq!(script_input_spelling(&path), "/lib/libc.so.6");
        assert_eq!(script_input_spelling(&lib), "-lgcc_s");
    }

    #[test]
    fn static_only_never_picks_a_shared_object() {
        let root = std::env::temp_dir().join(format!("lccc_resolve_lib_{}", std::process::id()));
        let (d1, d2) = (root.join("d1"), root.join("d2"));
        std::fs::create_dir_all(&d1).unwrap();
        std::fs::create_dir_all(&d2).unwrap();
        std::fs::write(d1.join("libfoo.so"), b"").unwrap();
        std::fs::write(d2.join("libfoo.a"), b"").unwrap();
        std::fs::write(d2.join("libfoo.so"), b"").unwrap();
        let paths = vec![d1.display().to_string(), d2.display().to_string()];
        let got = |static_only| resolve_lib_positional("foo", &paths, static_only);
        assert_eq!(got(false), Some(format!("{}/libfoo.so", paths[0])));
        assert_eq!(got(true), Some(format!("{}/libfoo.a", paths[1])));
        assert_eq!(resolve_lib_positional("bar", &paths, true), None);
        std::fs::remove_file(d2.join("libfoo.a")).unwrap();
        assert_eq!(got(true), None, "-Bstatic must not fall back to libfoo.so");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn file_mode_libs_expand_in_place_and_reject_shared_objects() {
        let root = std::env::temp_dir().join(format!("lccc_file_mode_libs_{}", std::process::id()));
        let (lib, extra) = (root.join("lib"), root.join("extra"));
        std::fs::create_dir_all(&lib).unwrap();
        std::fs::create_dir_all(&extra).unwrap();
        let mut rel = vec![
            0x7f, b'E', b'L', b'F', 2, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0,
        ];
        std::fs::write(lib.join("liba.a"), b"!<arch>\n").unwrap();
        std::fs::write(extra.join("libb.a"), b"!<arch>\n").unwrap();
        std::fs::write(lib.join("obj.o"), &rel).unwrap();
        rel[16] = 3; // ET_DYN
        std::fs::write(lib.join("libc1.so"), &rel).unwrap();
        std::fs::write(lib.join("libc1.a"), b"!<thin>\n").unwrap();
        let a = root.join("a.o").display().to_string();
        let z = root.join("z.o").display().to_string();
        std::fs::write(&a, b"").unwrap();
        std::fs::write(&z, b"").unwrap();
        let (l, x) = (lib.display().to_string(), extra.display().to_string());
        let args: Vec<String> = [
            a.as_str(),
            "-la",
            "--whole-archive",
            "-lb",
            "--no-whole-archive",
            "-Bstatic",
            "-lc1",
            "-Bdynamic",
            "-l:obj.o",
            z.as_str(),
        ]
        .iter()
        .map(|s| s.to_string())
        .chain([format!("-L{l}")])
        .collect();
        let files = vec![
            (a.clone(), false),
            (z.clone(), true),
            ("gone.o".to_string(), false),
        ];
        let got = expand_file_mode_libs(&args, &files, std::slice::from_ref(&x), "-r").unwrap();
        assert_eq!(
            got,
            vec![
                (a.clone(), false),
                (format!("{l}/liba.a"), false),
                (format!("{x}/libb.a"), true),
                (format!("{l}/libc1.a"), false),
                (format!("{l}/obj.o"), false),
                (z.clone(), true),
                ("gone.o".to_string(), false),
            ]
        );
        let shared = vec![format!("-L{l}"), "-lc1".to_string()];
        let err = expand_file_mode_libs(&shared, &[], &[], "-T").unwrap_err();
        assert!(err.contains("libc1.so") && err.contains("-T"), "{err}");
        let err = expand_file_mode_libs(&["-lnope".to_string()], &[], &[], "-r").unwrap_err();
        assert_eq!(err, "cannot find -lnope");
        let _ = std::fs::remove_dir_all(&root);
    }
}
