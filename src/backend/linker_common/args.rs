//! Shared linker argument parsing.
//!
//! Extracts linker flags from the `user_args` passed through `-Wl,` and
//! direct `-L`/`-l` flags. Used by x86, ARM, and RISC-V linkers.

use std::path::Path;

/// Parsed linker arguments from user_args.
///
/// Contains all the flags that are common across backends. Not all backends
/// use every field; unused fields are simply ignored.
#[derive(Debug, Default)]
pub struct LinkerArgs {
    /// Extra library search paths from `-L` flags.
    pub extra_lib_paths: Vec<String>,
    /// Library names from `-l` flags (without the `lib` prefix or `.a`/`.so` suffix).
    pub libs_to_load: Vec<String>,
    /// Bare file paths (`.o`, `.a` files) passed as arguments.
    pub extra_object_files: Vec<String>,
    /// Whether `--export-dynamic` / `-rdynamic` was passed.
    pub export_dynamic: bool,
    /// RPATH entries from `-Wl,-rpath=` or `-Wl,-rpath,`.
    pub rpath_entries: Vec<String>,
    /// Use DT_RUNPATH instead of DT_RPATH (from `--enable-new-dtags`).
    pub use_runpath: bool,
    /// Symbol definitions from `--defsym=SYM=VAL`.
    /// TODO: only supports symbol-to-symbol aliasing, not arbitrary expressions.
    pub defsym_defs: Vec<(String, String)>,
    /// Enable garbage collection of unused sections (from `--gc-sections`).
    pub gc_sections: bool,
    /// Whether `-static` was passed.
    pub is_static: bool,
    /// Entry point symbol from `-e SYM` / `--entry=SYM` (default `_start`).
    pub entry_symbol: Option<String>,
    /// Symbols to wrap from `--wrap=SYM`: references to SYM are redirected to
    /// `__wrap_SYM`, and references to `__real_SYM` are redirected to SYM.
    pub wrap_symbols: Vec<String>,
    /// Symbols forced undefined via `-u SYM` / `--undefined=SYM` (forces
    /// archive members that define them to be pulled in).
    pub undefined_symbols: Vec<String>,
    /// `-z now`: eager binding (DT_FLAGS: BIND_NOW, DT_FLAGS_1: NOW).
    pub z_now: bool,
    /// `-z relro` (default true, `-z norelro` clears): emit PT_GNU_RELRO.
    pub z_relro: bool,
    /// `-Map=FILE` / `-Map FILE`: write a GNU-ld-compatible link map.
    pub map_path: Option<String>,
    /// `--exclude-libs=LIST`: archives whose symbols must NOT be re-exported
    /// from a shared library. Comma/colon-separated basenames, or `ALL`.
    pub exclude_libs: Vec<String>,
    /// `--version-script=FILE`: restricts the exported symbol set.
    pub version_script: Option<String>,
    /// `-soname=NAME`: DT_SONAME recorded in a shared library.
    pub soname: Option<String>,
    /// `-Bsymbolic` / `-Bsymbolic-functions`: bind global references inside a
    /// shared library to its own definitions.
    pub bsymbolic: bool,
    /// `--no-undefined` / `-z defs`: reject unresolved symbols in a shared
    /// library instead of deferring them to the loader.
    pub no_undefined: bool,
    /// Inputs in **command-line order**, each tagged with the positional state
    /// in effect where it appeared.
    ///
    /// `extra_object_files` / `libs_to_load` above are order-insensitive views
    /// kept for existing callers. They cannot express `--whole-archive`, which
    /// is *positional*: it applies only to archives that follow it, until
    /// `--no-whole-archive`. Link order also decides archive member selection,
    /// so any caller doing real archive resolution must use this list.
    pub inputs: Vec<InputItem>,
}

/// One input file or `-l` library, with the positional flag state that applied
/// at the point it appeared on the command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputItem {
    /// Either a bare path (`foo.o`, `libbar.a`) or a library stem from `-lfoo`.
    pub name: String,
    /// True when this came from `-l`/`--library` and still needs `-L` search.
    pub is_lib: bool,
    /// True when `--whole-archive` was in effect: every member of this archive
    /// is linked in, not just those resolving an undefined symbol.
    pub whole_archive: bool,
}

/// Parse user linker arguments into a structured `LinkerArgs`.
///
/// Handles `-L`, `-l`, `-Wl,` (with nested flags like `--defsym`, `--export-dynamic`,
/// `-rpath`, `--gc-sections`), `-rdynamic`, `-static`, and bare file paths.
pub fn parse_linker_args(user_args: &[String]) -> LinkerArgs {
    let mut result = LinkerArgs::default();
    result.z_relro = true; // RELRO is on by default, like GNU ld/mold
    let args: Vec<&str> = user_args.iter().map(|s| s.as_str()).collect();
    let mut pending_rpath = false; // for -Wl,-rpath -Wl,/path two-arg form
    // Positional state: --whole-archive applies to archives that FOLLOW it,
    // until --no-whole-archive turns it back off.
    let mut whole_archive = false;
    let mut i = 0;
    while i < args.len() {
        let arg = args[i];
        if arg == "-rdynamic" {
            result.export_dynamic = true;
        } else if let Some(v) = arg.strip_prefix("--version-script=") {
            result.version_script = Some(v.to_string());
        } else if arg == "--version-script" && i + 1 < args.len() {
            i += 1;
            result.version_script = Some(args[i].to_string());
        } else if let Some(v) = arg.strip_prefix("--exclude-libs=") {
            result.exclude_libs.extend(split_lib_list(v));
        } else if arg == "--exclude-libs" && i + 1 < args.len() {
            i += 1;
            result.exclude_libs.extend(split_lib_list(args[i]));
        } else if let Some(v) = arg.strip_prefix("-Map=") {
            // Top-level spelling: lccc-ld and other direct callers pass
            // `-Map=FILE` as its own argument, not inside a `-Wl,` group.
            result.map_path = Some(v.to_string());
        } else if arg == "-Map" && i + 1 < args.len() {
            i += 1;
            result.map_path = Some(args[i].to_string());
        } else if arg == "-static" {
            result.is_static = true;
        } else if let Some(path) = arg.strip_prefix("-L") {
            let p = if path.is_empty() && i + 1 < args.len() { i += 1; args[i] } else { path };
            result.extra_lib_paths.push(p.to_string());
        } else if let Some(lib) = arg.strip_prefix("-l") {
            let l = if lib.is_empty() && i + 1 < args.len() { i += 1; args[i] } else { lib };
            result.libs_to_load.push(l.to_string());
            result.inputs.push(InputItem {
                name: l.to_string(), is_lib: true, whole_archive,
            });
        } else if let Some(wl_arg) = arg.strip_prefix("-Wl,") {
            let parts: Vec<&str> = wl_arg.split(',').collect();
            // Handle -Wl,-rpath -Wl,/path two-arg form
            if pending_rpath && !parts.is_empty() {
                result.rpath_entries.push(parts[0].to_string());
                pending_rpath = false;
                i += 1;
                continue;
            }
            let mut j = 0;
            while j < parts.len() {
                let part = parts[j];
                if part == "--export-dynamic" || part == "-export-dynamic" || part == "-E" {
                    result.export_dynamic = true;
                } else if let Some(rp) = part.strip_prefix("-rpath=") {
                    result.rpath_entries.push(rp.to_string());
                } else if part == "-rpath" && j + 1 < parts.len() {
                    j += 1;
                    result.rpath_entries.push(parts[j].to_string());
                } else if part == "-rpath" {
                    // -rpath without following value in this -Wl, group;
                    // the path comes in the next -Wl, argument
                    pending_rpath = true;
                } else if part == "--enable-new-dtags" {
                    result.use_runpath = true;
                } else if part == "--disable-new-dtags" {
                    result.use_runpath = false;
                } else if let Some(lpath) = part.strip_prefix("-L") {
                    result.extra_lib_paths.push(lpath.to_string());
                } else if let Some(lib) = part.strip_prefix("-l") {
                    result.libs_to_load.push(lib.to_string());
                    result.inputs.push(InputItem {
                        name: lib.to_string(), is_lib: true, whole_archive,
                    });
                } else if let Some(defsym_arg) = part.strip_prefix("--defsym=") {
                    if let Some(eq_pos) = defsym_arg.find('=') {
                        result.defsym_defs.push((
                            defsym_arg[..eq_pos].to_string(),
                            defsym_arg[eq_pos + 1..].to_string(),
                        ));
                    }
                } else if part == "--defsym" && j + 1 < parts.len() {
                    j += 1;
                    let defsym_arg = parts[j];
                    if let Some(eq_pos) = defsym_arg.find('=') {
                        result.defsym_defs.push((
                            defsym_arg[..eq_pos].to_string(),
                            defsym_arg[eq_pos + 1..].to_string(),
                        ));
                    }
                } else if let Some(v) = part.strip_prefix("--version-script=") {
                    result.version_script = Some(v.to_string());
                } else if part == "--version-script" && j + 1 < parts.len() {
                    j += 1;
                    result.version_script = Some(parts[j].to_string());
                } else if let Some(v) = part.strip_prefix("--exclude-libs=") {
                    result.exclude_libs.extend(split_lib_list(v));
                } else if part == "--exclude-libs" && j + 1 < parts.len() {
                    j += 1;
                    result.exclude_libs.extend(split_lib_list(parts[j]));
                } else if let Some(v) = part.strip_prefix("-Map=") {
                    result.map_path = Some(v.to_string());
                } else if part == "-Map" && j + 1 < parts.len() {
                    j += 1;
                    result.map_path = Some(parts[j].to_string());
                } else if part == "--gc-sections" {
                    result.gc_sections = true;
                } else if part == "--no-gc-sections" {
                    result.gc_sections = false;
                } else if part == "-static" {
                    result.is_static = true;
                } else if part == "-z" && j + 1 < parts.len() {
                    j += 1;
                    match parts[j] {
                        "now" => result.z_now = true,
                        "lazy" => result.z_now = false,
                        "relro" => result.z_relro = true,
                        "norelro" => result.z_relro = false,
                        // `-z defs` is the spelling CMake/Qt use for
                        // --no-undefined. It must be handled *here*, in the
                        // single `-z` arm: a later `else if part == "-z" ...`
                        // branch is unreachable because this one already
                        // matched and consumed the keyword. That exact trap
                        // silently disabled -z defs for shared libraries when
                        // link_shared's private parser was removed, and the
                        // so_z_defs_rejects_undefined differential test caught it.
                        "defs" => result.no_undefined = true,
                        "undefs" => result.no_undefined = false,
                        _ => {} // noexecstack, origin, ... not layout-affecting
                    }
                } else if let Some(sym) = part.strip_prefix("--entry=") {
                    result.entry_symbol = Some(sym.to_string());
                } else if (part == "-e" || part == "--entry") && j + 1 < parts.len() {
                    j += 1;
                    result.entry_symbol = Some(parts[j].to_string());
                } else if let Some(sym) = part.strip_prefix("--wrap=") {
                    result.wrap_symbols.push(sym.to_string());
                } else if part == "--wrap" && j + 1 < parts.len() {
                    j += 1;
                    result.wrap_symbols.push(parts[j].to_string());
                } else if let Some(sym) = part.strip_prefix("--undefined=") {
                    result.undefined_symbols.push(sym.to_string());
                } else if (part == "-u" || part == "--undefined") && j + 1 < parts.len() {
                    j += 1;
                    result.undefined_symbols.push(parts[j].to_string());
                } else if let Some(sym) = part.strip_prefix("-u") {
                    if !sym.is_empty() && !sym.starts_with('-') {
                        result.undefined_symbols.push(sym.to_string());
                    }
                } else if let Some(sym) = part.strip_prefix("-e") {
                    // -e<sym> joined form (only if it looks like a symbol, not
                    // another flag such as -export-dynamic which is handled above)
                    if !sym.is_empty() && !sym.starts_with('-')
                        && !part.starts_with("-enable") && !part.starts_with("-export") {
                        result.entry_symbol = Some(sym.to_string());
                    }
                }
                else if let Some(sn) = part.strip_prefix("-soname=") {
                    result.soname = Some(sn.to_string());
                } else if part == "-soname" && j + 1 < parts.len() {
                    j += 1;
                    result.soname = Some(parts[j].to_string());
                } else if part == "-Bsymbolic" || part == "-Bsymbolic-functions" {
                    result.bsymbolic = true;
                } else if part == "--no-undefined" {
                    result.no_undefined = true;
                } else if part == "--whole-archive" {
                    whole_archive = true;
                } else if part == "--no-whole-archive" {
                    whole_archive = false;
                }
                j += 1;
            }
        } else if let Some(sn) = arg.strip_prefix("-soname=") {
            result.soname = Some(sn.to_string());
        } else if arg == "-soname" && i + 1 < args.len() {
            i += 1;
            result.soname = Some(args[i].to_string());
        } else if arg == "-Bsymbolic" || arg == "-Bsymbolic-functions" {
            result.bsymbolic = true;
        } else if arg == "--no-undefined" {
            result.no_undefined = true;
        } else if arg == "--whole-archive" {
            whole_archive = true;
        } else if arg == "--no-whole-archive" {
            whole_archive = false;
        } else if !arg.starts_with('-') && Path::new(arg).exists() {
            result.extra_object_files.push(arg.to_string());
            result.inputs.push(InputItem {
                name: arg.to_string(), is_lib: false, whole_archive,
            });
        }
        i += 1;
    }
    result
}

#[cfg(test)]
mod map_arg_tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    /// `-Map` reaches the linker in four spellings depending on whether the
    /// caller is a Makefile driving `ld` directly, gcc forwarding via `-Wl,`,
    /// or a build system using the separated form. All four must work.
    #[test]
    fn map_path_is_recognised_in_every_spelling() {
        assert_eq!(parse_linker_args(&args(&["-Map=out.map"])).map_path.as_deref(),
                   Some("out.map"));
        assert_eq!(parse_linker_args(&args(&["-Map", "out.map"])).map_path.as_deref(),
                   Some("out.map"));
        assert_eq!(parse_linker_args(&args(&["-Wl,-Map=out.map"])).map_path.as_deref(),
                   Some("out.map"));
        assert_eq!(parse_linker_args(&args(&["-Wl,-Map,out.map"])).map_path.as_deref(),
                   Some("out.map"));
    }

    #[test]
    fn map_path_absent_by_default() {
        assert!(parse_linker_args(&args(&["-static", "a.o"])).map_path.is_none());
    }

    /// A path containing '=' (legal) must survive intact.
    #[test]
    fn map_path_with_equals_in_filename() {
        assert_eq!(parse_linker_args(&args(&["-Map=/tmp/a=b.map"])).map_path.as_deref(),
                   Some("/tmp/a=b.map"));
    }
}

/// Split an `--exclude-libs` value into archive names.
///
/// GNU ld accepts both comma and colon as separators (`--exclude-libs
/// libfoo.a:libbar.a` and `...=libfoo.a,libbar.a` are both in the wild), and
/// the special value `ALL`. Entries are stored verbatim; matching happens in
/// `exclude_libs_matches`.
fn split_lib_list(v: &str) -> Vec<String> {
    v.split([',', ':'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

/// Does `source_name` (e.g. `/usr/lib/libfoo.a(bar.o)`) belong to an archive
/// named by `--exclude-libs`?
///
/// GNU ld matches on the archive's *basename*, not its path, and `ALL` matches
/// every static archive. Objects that did not come from an archive (no
/// parenthesised member) are never excluded — `--exclude-libs` is about
/// archives only.
pub fn exclude_libs_matches(exclude: &[String], source_name: &str) -> bool {
    if exclude.is_empty() {
        return false;
    }
    // Archive members are recorded as "path/to/libfoo.a(member.o)".
    let Some(paren) = source_name.find('(') else { return false };
    if !source_name.ends_with(')') {
        return false;
    }
    let archive = &source_name[..paren];
    let base = archive.rsplit('/').next().unwrap_or(archive);
    exclude.iter().any(|e| {
        e.eq_ignore_ascii_case("ALL") || e == base || e == archive
    })
}

#[cfg(test)]
mod ordered_input_tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    /// Create real files, because `parse_linker_args` only accepts bare paths
    /// that exist on disk (it must not mistake a stray token for an input).
    fn with_files<T>(names: &[&str], f: impl FnOnce(&std::path::Path) -> T) -> T {
        let td = std::env::temp_dir().join(format!("lccc_args_{}", std::process::id()));
        std::fs::create_dir_all(&td).unwrap();
        for n in names {
            std::fs::write(td.join(n), b"").unwrap();
        }
        let r = f(&td);
        let _ = std::fs::remove_dir_all(&td);
        r
    }

    /// `--whole-archive` is *positional*: it applies to archives that follow
    /// it and stops at `--no-whole-archive`. Recording it as a single global
    /// bool (what `LinkerArgs` could express before `inputs` existed) links
    /// every member of every archive, which silently bloats output and can
    /// introduce duplicate-symbol errors.
    #[test]
    fn whole_archive_is_positional_not_global() {
        with_files(&["a.a", "b.a", "c.a"], |d| {
            let p = |n: &str| d.join(n).to_string_lossy().to_string();
            let r = parse_linker_args(&args(&[
                &p("a.a"),
                "--whole-archive",
                &p("b.a"),
                "--no-whole-archive",
                &p("c.a"),
            ]));
            let got: Vec<(String, bool)> = r.inputs.iter()
                .map(|i| (
                    std::path::Path::new(&i.name)
                        .file_name().unwrap().to_string_lossy().to_string(),
                    i.whole_archive,
                ))
                .collect();
            assert_eq!(got, vec![
                ("a.a".to_string(), false),
                ("b.a".to_string(), true),
                ("c.a".to_string(), false),
            ]);
        });
    }

    /// The same must hold through the `-Wl,` spelling, which is how gcc
    /// actually passes these flags.
    #[test]
    fn whole_archive_positional_via_wl() {
        with_files(&["x.a", "y.a"], |d| {
            let p = |n: &str| d.join(n).to_string_lossy().to_string();
            let r = parse_linker_args(&args(&[
                "-Wl,--whole-archive",
                &p("x.a"),
                "-Wl,--no-whole-archive",
                &p("y.a"),
            ]));
            let got: Vec<bool> = r.inputs.iter().map(|i| i.whole_archive).collect();
            assert_eq!(got, vec![true, false]);
        });
    }

    /// Link order decides archive member selection, so `inputs` must preserve
    /// the exact command-line sequence, interleaving `-l` libs and bare files.
    #[test]
    fn inputs_preserve_command_line_order() {
        with_files(&["first.o", "second.o"], |d| {
            let p = |n: &str| d.join(n).to_string_lossy().to_string();
            let r = parse_linker_args(&args(&[
                &p("first.o"), "-lm", &p("second.o"), "-lpthread",
            ]));
            let got: Vec<(String, bool)> = r.inputs.iter()
                .map(|i| (
                    std::path::Path::new(&i.name)
                        .file_name().unwrap().to_string_lossy().to_string(),
                    i.is_lib,
                ))
                .collect();
            assert_eq!(got, vec![
                ("first.o".to_string(), false),
                ("m".to_string(), true),
                ("second.o".to_string(), false),
                ("pthread".to_string(), true),
            ]);
        });
    }

    /// `inputs` is an additional view, not a replacement: the pre-existing
    /// order-insensitive fields must keep working so current callers are
    /// unaffected by the refactor.
    #[test]
    fn legacy_views_still_populated() {
        with_files(&["o1.o"], |d| {
            let p = |n: &str| d.join(n).to_string_lossy().to_string();
            let r = parse_linker_args(&args(&[&p("o1.o"), "-lfoo", "-Wl,-lbar"]));
            assert_eq!(r.libs_to_load, vec!["foo".to_string(), "bar".to_string()]);
            assert_eq!(r.extra_object_files.len(), 1);
            assert_eq!(r.inputs.len(), 3, "one entry per input, in order");
        });
    }
}
