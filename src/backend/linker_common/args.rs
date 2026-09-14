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
    /// Emit DT_RUNPATH instead of DT_RPATH.  Defaults to true (bfd/lld/mold
    /// parity); `--disable-new-dtags` clears it, `--enable-new-dtags` sets it.
    pub use_runpath: bool,
    /// Sort SHN_COMMON symbols by descending size before BSS allocation
    /// (from `--sort-common[=yes]`; `--sort-common=no` / `--no-sort-common`
    /// restore the default hash order).  Off by default, as in bfd.
    pub sort_common: bool,
    /// Symbol definitions from `--defsym=SYM=VAL`.
    /// TODO: only supports symbol-to-symbol aliasing, not arbitrary expressions.
    pub defsym_defs: Vec<(String, String)>,
    /// Enable garbage collection of unused sections (from `--gc-sections`).
    pub gc_sections: bool,
    /// Identical Code Folding mode from `--icf=safe|all|none`.
    ///
    /// `None` disables ICF. `Some("safe")` refuses to fold functions whose
    /// address is observable; `Some("all")` folds regardless, which is only
    /// valid when the program never compares function pointers.
    pub icf: Option<String>,
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
    /// Requested ELF interpreter from `--dynamic-linker=PATH`.
    ///
    /// The default remains the target ABI interpreter.  Keeping this in the
    /// parsed argument object is important for sysroot/staging builds (and for
    /// testing a freshly built libc without replacing the host loader):
    /// silently hard-wiring `/lib64/ld-linux-x86-64.so.2` makes an otherwise
    /// valid binary execute against the wrong libc.
    pub dynamic_linker: Option<String>,
    /// `-soname=NAME`: DT_SONAME recorded in a shared library.
    pub soname: Option<String>,
    /// `--build-id[=STYLE]`: emit `.note.gnu.build-id`.
    ///
    /// Not a nicety.  Debian's gcc passes `--build-id` on *every* link, and
    /// debuginfod, RPM/DEB debuginfo extraction, systemd-coredump matching and
    /// `eu-unstrip` all key on it.  Silently honouring the flag while emitting
    /// no note produces binaries that look linked but cannot be symbolised
    /// after the fact.  `--build-id=none` (and `=0`) clears it, matching GNU ld.
    pub build_id: bool,
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
    /// `-pie` / `--pic-executable`: emit a position-independent executable
    /// (`ET_DYN` based at 0, `DF_1_PIE`, `R_X86_64_RELATIVE` for internal
    /// absolute addresses).  Debian's gcc passes `-pie` on every link, so a
    /// linker that ignores it silently ships every binary without ASLR.
    pub is_pie: bool,
    /// `-s` / `--strip-all`: omit `.symtab` and `.strtab` from the output.
    pub strip_all: bool,
    /// `--hash-style=gnu|sysv|both`.  `Gnu` is the default everywhere that
    /// matters on x86-64; `both` is still requested by builds targeting
    /// pre-2.23 glibc, which has no `DT_GNU_HASH` support.
    pub hash_style: HashStyle,
    /// `-z ibt`: indirect-branch tracking.  Sets the `GNU_PROPERTY_X86_
    /// FEATURE_1_AND` IBT bit (1) in the merged property note, creating the
    /// note when no input had one (GNU ld behaviour).
    pub z_ibt: bool,
    /// `-z shstk`: shadow-stack control (FEATURE_1_AND bit 2).
    pub z_shstk: bool,
    /// `-z lam-u48`: 48-bit linear-address masking (FEATURE_1_AND bits 4|8).
    pub z_lam_u48: bool,
    /// `-z lam-u57`: 57-bit linear-address masking (FEATURE_1_AND bit 8).
    pub z_lam_u57: bool,
    /// `-z x86-64-{baseline,v2,v3,v4}` ISA level (binutils
    /// `ld/emulparams/x86-64-level.sh`): 0 = unset (the default — nothing is
    /// injected), 1 = baseline, 2/3/4 = v2/v3/v4.  ORed into
    /// `GNU_PROPERTY_X86_ISA_1_NEEDED` by the property-note merge (see
    /// `linker_common::cet`), creating the property when no input had one.
    pub z_isa_level: u32,
    /// An unparseable `-z x86-64-*` keyword (e.g. `-z x86-64-v9`), kept so
    /// the link path can fail with GNU's `invalid x86-64 ISA level` fatal
    /// instead of silently ignoring the typo.
    pub z_isa_level_invalid: Option<String>,
    /// `-z ibt=func` & co: keyword forms GNU ld accepts and ignores with a
    /// warning.  Collected here so the driver can print the warning once.
    pub z_ignored_keywords: Vec<String>,
    /// `--emit-relocs` / `-q`: keep the applied relocations in `.rela.*`
    /// sections (the kernel's arch/x86/tools/relocs pass consumes them).
    pub emit_relocs: bool,
    /// `--threads=N`: recorded for interface compatibility; lccc links
    /// single-threaded, so the value is accepted and not acted on.
    #[allow(dead_code)]
    pub threads: Option<u32>,
    /// `--no-undefined-version`: recorded for interface compatibility (GNU
    /// ld "Disallow undefined version").  lccc accepts the flag but does
    /// not enforce it: version references are never checked against the
    /// version definitions, so the link succeeds where GNU ld would fail
    /// an unsatisfiable version reference.  Same accepted-but-recorded
    /// status as `threads`; enforcement needs version-script checking the
    /// linker does not have yet.
    #[allow(dead_code)]
    pub no_undefined_version: bool,
}

impl LinkerArgs {
    /// The GNU property-note merge flags (`linker_common::cet`) for this
    /// link: CET bits plus the `-z x86-64-*` ISA level.  Fails on an
    /// unparseable ISA level with GNU's message body (`invalid x86-64 ISA
    /// level: ...`, measured on ld 2.44); the driver renders it with its
    /// standard `lccc-ld: error: ` prefix like every other link failure.
    pub fn property_link_flags(&self) -> Result<super::cet::PropertyLinkFlags, String> {
        if let Some(bad) = &self.z_isa_level_invalid {
            return Err(format!("invalid x86-64 ISA level: {bad}"));
        }
        Ok(super::cet::PropertyLinkFlags {
            ibt: self.z_ibt,
            shstk: self.z_shstk,
            lam_u48: self.z_lam_u48,
            lam_u57: self.z_lam_u57,
            isa_level: self.z_isa_level,
        })
    }
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
    /// True when `--as-needed` was in effect at this position.
    ///
    /// Like `--whole-archive`, this is a *positional* toggle: it applies to the
    /// inputs that follow it, until `--no-as-needed`. A shared library tagged
    /// as-needed only gets a `DT_NEEDED` entry when it actually resolves
    /// something; one tagged no-as-needed is recorded unconditionally, which is
    /// how a program pulls in a library purely for its ELF constructors.
    pub as_needed: bool,
}

/// Which dynamic symbol hash tables to emit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HashStyle {
    /// `.gnu.hash` + `DT_GNU_HASH` only.  The default.
    #[default]
    Gnu,
    /// `.hash` + `DT_HASH` only.
    Sysv,
    /// Both tables, for loaders too old for `DT_GNU_HASH`.
    Both,
}

impl HashStyle {
    pub fn wants_gnu(self) -> bool {
        matches!(self, HashStyle::Gnu | HashStyle::Both)
    }
    pub fn wants_sysv(self) -> bool {
        matches!(self, HashStyle::Sysv | HashStyle::Both)
    }
}

/// Parse the value of `--hash-style=`.
///
/// GNU ld accepts `gnu`, `sysv`, `both`, and a comma-separated combination
/// (`gnu,sysv`).  Returns `None` for an unrecognised value so the caller can
/// keep the default rather than silently guessing.
pub fn parse_hash_style(v: &str) -> Option<HashStyle> {
    let mut gnu = false;
    let mut sysv = false;
    for tok in v.to_ascii_lowercase().split(',') {
        match tok.trim() {
            "gnu" => gnu = true,
            "sysv" => sysv = true,
            "both" => {
                gnu = true;
                sysv = true;
            }
            _ => return None,
        }
    }
    Some(match (gnu, sysv) {
        (false, true) => HashStyle::Sysv,
        (true, true) => HashStyle::Both,
        _ => HashStyle::Gnu,
    })
}

/// Apply a single linker flag that is a plain token-to-field mapping.
///
/// Returns `true` if the token was consumed.  Both the top-level argument loop
/// and the `-Wl,` sub-argument loop call this, so a flag spelled either way is
/// handled by one definition instead of two that can drift.  Flags needing a
/// value or carrying positional state are handled inline by the callers.
/// Options whose spelling is a single token: relocation retention and
/// thread hints.  Returns true when consumed.
///
/// Deliberately NOT here: `--isa-level=` / `-march=`.  GNU ld has no such
/// options (it errors on them); the real ISA-level interface is
/// `-z x86-64-{baseline,v2,v3,v4}`, parsed with the other `-z` keywords
/// below.  Accepting lookalike spellings would bless a non-GNU interface.
fn misc_option(result: &mut LinkerArgs, tok: &str) -> bool {
    match tok {
        "--emit-relocs" | "-q" => {
            result.emit_relocs = true;
        }
        "--no-emit-relocs" => {
            result.emit_relocs = false;
        }
        "--no-undefined-version" => {
            result.no_undefined_version = true;
        }
        "--sort-common" => {
            result.sort_common = true;
        }
        "--no-sort-common" => {
            result.sort_common = false;
        }
        "--no-threads" => {
            result.threads = None;
        }
        "--threads" => {
            // The value arrives as the next argument on the plain path;
            // the group path joins it first (`--threads,N`).  Recording is
            // best-effort: lccc links single-threaded.
            return true;
        }
        _ => {
            if let Some(v) = tok.strip_prefix("--threads=") {
                // Lenient like GNU: a bad value is ignored, not fatal.
                result.threads = v.parse().ok();
                return true;
            }
            if let Some(v) = tok.strip_prefix("--sort-common=") {
                // Lenient like GNU: an unknown value keeps the current
                // setting instead of erroring the whole link.
                match v {
                    "yes" => result.sort_common = true,
                    "no" => result.sort_common = false,
                    _ => {}
                }
                return true;
            }
            return false;
        }
    }
    true
}

fn apply_plain_flag(result: &mut LinkerArgs, tok: &str) -> bool {
    match tok {
        "-pie" | "--pie" | "--pic-executable" => result.is_pie = true,
        "-no-pie" | "--no-pie" | "--no-pic-executable" => result.is_pie = false,
        // Exact match only, so `-soname`/`-shared` are not shadowed.
        "-s" | "--strip-all" => result.strip_all = true,
        _ => return false,
    }
    true
}

/// Parse user linker arguments into a structured `LinkerArgs`.
///
/// Handles `-L`, `-l`, `-Wl,` (with nested flags like `--defsym`, `--export-dynamic`,
/// `-rpath`, `--gc-sections`), `-rdynamic`, `-static`, and bare file paths.
/// Apply one `-z KEYWORD` to the parsed arguments.
///
/// Both the split form (`-Wl,-z,relro`) and the joined form
/// (`-Wl,-zrelro`) funnel through this one function so the two GNU
/// spellings cannot drift apart.  There must be exactly ONE such
/// function: `-z defs` (the spelling CMake/Qt use for --no-undefined)
/// was once handled in a second `-z` arm that the first arm's match had
/// already made unreachable, silently disabling it for shared
/// libraries until the so_z_defs_rejects_undefined differential test
/// caught it.
fn apply_z_keyword(result: &mut LinkerArgs, kw: &str) {
    match kw {
        "now" => result.z_now = true,
        "lazy" => result.z_now = false,
        "relro" => result.z_relro = true,
        "norelro" => result.z_relro = false,
        "defs" => result.no_undefined = true,
        "undefs" => result.no_undefined = false,
        // CET property note bits (see linker_common::cet).
        "ibt" => result.z_ibt = true,
        "shstk" => result.z_shstk = true,
        "lam-u48" => result.z_lam_u48 = true,
        "lam-u57" => result.z_lam_u57 = true,
        // x86-64 ISA level (binutils `ld/emulparams/x86-64-level.sh`):
        // `-z x86-64-{baseline,v2,v3,v4}` marks the level as needed in
        // the property note.  GNU's rule, measured on ld 2.44:
        //   * baseline / v2 / v3 / v4 set the level;
        //   * `x86-64-v` + a bad suffix (v, v1, v2x, v9, ...) is the
        //     `invalid x86-64 ISA level` fatal — the parser has no error
        //     channel, so the keyword is kept for the link path to
        //     reject via `property_link_flags`;
        //   * any OTHER `x86-64-*` keyword (x86-64-foo, x86-64-,
        //     x86-64-baselineX, wrong-case X86-64-v3) is warned about
        //     and ignored (`-z <kw> ignored`), like every other
        //     unrecognised `-z` keyword — NOT fatal.
        "x86-64-baseline" => result.z_isa_level = 1,
        k if k.starts_with("x86-64-v") => match k["x86-64-v".len()..].parse::<u32>() {
            Ok(n) if (2..=4).contains(&n) => result.z_isa_level = n,
            _ => result.z_isa_level_invalid = Some(k.to_string()),
        },
        k if k.starts_with("x86-64-") => {
            result.z_ignored_keywords.push(k.to_string());
        }
        k if k.starts_with("ibt=")
            || k.starts_with("shstk=")
            || k.starts_with("lam-u48=")
            || k.starts_with("lam-u57=") =>
        {
            // `-z ibt=func` & co: accepted, ignored, warned.
            result.z_ignored_keywords.push(k.to_string());
        }
        _ => {} // noexecstack, origin, ... not layout-affecting
    }
}

pub fn parse_linker_args(user_args: &[String]) -> LinkerArgs {
    let mut result = LinkerArgs::default();
    // DT_RUNPATH is the default, not DT_RPATH: bfd (as configured by every
    // major distro), lld and mold all emit RUNPATH for a plain `-rpath`.
    // Upstream bfd's historic RPATH default only survives in niche
    // configurations; matching the de-facto standard keeps
    // `-Wl,-rpath,$ORIGIN` links behaving identically under every linker.
    // `--disable-new-dtags` still opts back into DT_RPATH below.
    result.use_runpath = true;
    result.z_relro = true; // RELRO is on by default, like GNU ld/mold

    // GNU ld accepts `--opt VALUE` alongside `--opt=VALUE`.  A driver that
    // forwards them as `-Wl,--opt -Wl,VALUE` splits the pair across two
    // arguments, and the `-Wl,` splitter below can only see one group at a
    // time, so the value is silently dropped.  Re-join such a pair up front:
    // `-Wl,--wrap -Wl,bv` becomes `-Wl,--wrap,bv`, which the group parser
    // already handles.  This replaces the ad-hoc `pending_rpath` workaround
    // and covers every two-argument option at once.
    const VALUE_TAKING: &[&str] = &[
        "--wrap",
        "--defsym",
        "--icf",
        "-Map",
        "--version-script",
        "--exclude-libs",
        "-rpath",
        "--hash-style",
        "--dynamic-linker",
        "-dynamic-linker",
        "-I",
        "-soname",
        "--entry",
        "-e",
        "--undefined",
        "-u",
        "-z",
    ];
    let joined: Vec<String> = {
        let mut out: Vec<String> = Vec::with_capacity(user_args.len());
        let mut k = 0;
        while k < user_args.len() {
            let mut cur = user_args[k].clone();
            k += 1;
            // Absorb following -Wl, groups while this one ends in an option
            // that is still waiting for its value.
            while k < user_args.len() {
                let wl = match cur.strip_prefix("-Wl,") {
                    Some(w) => w,
                    None => break,
                };
                let last = match wl.split(',').next_back() {
                    Some(l) => l,
                    None => break,
                };
                if !VALUE_TAKING.contains(&last) {
                    break;
                }
                let nxt = match user_args[k].strip_prefix("-Wl,") {
                    Some(w) => w,
                    None => break, // value is a separate argv entry; leave it
                };
                cur = format!("{},{}", cur, nxt);
                k += 1;
            }
            out.push(cur);
        }
        out
    };
    let args: Vec<&str> = joined.iter().map(|s| s.as_str()).collect();
    // Retained for a `-Wl,-rpath` whose path arrives as a *separate argv
    // entry* (not another `-Wl,` group), which the join above deliberately
    // does not merge.
    let mut pending_rpath = false;
    // Positional state: --whole-archive applies to archives that FOLLOW it,
    // until --no-whole-archive turns it back off.
    let mut whole_archive = false;
    // GNU ld defaults to --no-as-needed; gcc's driver passes --as-needed
    // explicitly when it wants it.
    let mut as_needed = false;
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
        } else if arg == "--print-map" || arg == "-M" {
            result.map_path = Some("-".to_string());
        } else if let Some(v) = arg.strip_prefix("-Map=") {
            // Top-level spelling: lccc-ld and other direct callers pass
            // `-Map=FILE` as its own argument, not inside a `-Wl,` group.
            result.map_path = Some(v.to_string());
        } else if arg == "-Map" && i + 1 < args.len() {
            i += 1;
            result.map_path = Some(args[i].to_string());
        } else if let Some(v) = arg.strip_prefix("--dynamic-linker=") {
            if !v.is_empty() {
                result.dynamic_linker = Some(v.to_string());
            }
        } else if arg == "--dynamic-linker" || arg == "-dynamic-linker" || arg == "-I" {
            if i + 1 < args.len() {
                i += 1;
                if !args[i].is_empty() {
                    result.dynamic_linker = Some(args[i].to_string());
                }
            }
        } else if arg == "-static" {
            result.is_static = true;
        } else if let Some(path) = arg.strip_prefix("-L") {
            let p = if path.is_empty() && i + 1 < args.len() {
                i += 1;
                args[i]
            } else {
                path
            };
            result.extra_lib_paths.push(p.to_string());
        } else if let Some(lib) = arg.strip_prefix("-l") {
            let l = if lib.is_empty() && i + 1 < args.len() {
                i += 1;
                args[i]
            } else {
                lib
            };
            result.libs_to_load.push(l.to_string());
            result.inputs.push(InputItem {
                name: l.to_string(),
                is_lib: true,
                whole_archive,
                as_needed,
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
                } else if let Some(v) = part.strip_prefix("--dynamic-linker=") {
                    if !v.is_empty() {
                        result.dynamic_linker = Some(v.to_string());
                    }
                } else if let Some(v) = part.strip_prefix("-dynamic-linker=") {
                    if !v.is_empty() {
                        result.dynamic_linker = Some(v.to_string());
                    }
                } else if (part == "--dynamic-linker" || part == "-dynamic-linker" || part == "-I")
                    && j + 1 < parts.len()
                {
                    j += 1;
                    if !parts[j].is_empty() {
                        result.dynamic_linker = Some(parts[j].to_string());
                    }
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
                        name: lib.to_string(),
                        is_lib: true,
                        whole_archive,
                        as_needed,
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
                } else if part == "--print-map" || part == "-M" {
                    // GNU ld writes the map to stdout; "-" is the sentinel.
                    result.map_path = Some("-".to_string());
                } else if let Some(v) = part.strip_prefix("-Map=") {
                    result.map_path = Some(v.to_string());
                } else if part == "-Map" && j + 1 < parts.len() {
                    j += 1;
                    result.map_path = Some(parts[j].to_string());
                } else if let Some(v) = part.strip_prefix("--icf=") {
                    // Unknown values disable ICF rather than erroring, which
                    // matches how gold/lld treat --icf=none.
                    result.icf = crate::backend::x86::linker::parse_icf_mode(v).map(str::to_string);
                } else if part == "--no-icf" {
                    result.icf = None;
                } else if part == "--gc-sections" {
                    result.gc_sections = true;
                } else if part == "--no-gc-sections" {
                    result.gc_sections = false;
                } else if part == "-static" {
                    result.is_static = true;
                } else if part == "-z" && j + 1 < parts.len() {
                    j += 1;
                    apply_z_keyword(&mut result, parts[j]);
                } else if let Some(kw) = part.strip_prefix("-z") {
                    // Joined form inside the group (`-Wl,-zrelro`): GNU ld
                    // accepts `-z<keyword>` identically to `-z <keyword>`
                    // (measured on 2.44, including through `-Wl,`, which
                    // the compiler driver splits into the joined ld
                    // argument), so it funnels through the same keyword
                    // function.  A bare `-z` never reaches here — the arm
                    // above consumes it — but an empty remainder is
                    // skipped rather than asserted on.
                    if !kw.is_empty() {
                        apply_z_keyword(&mut result, kw);
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
                    if !sym.is_empty()
                        && !sym.starts_with('-')
                        && !part.starts_with("-enable")
                        && !part.starts_with("-export")
                    {
                        result.entry_symbol = Some(sym.to_string());
                    }
                } else if let Some(sn) = part.strip_prefix("-soname=") {
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
                } else if part == "--as-needed" {
                    as_needed = true;
                } else if part == "--no-as-needed" {
                    as_needed = false;
                } else if let Some(v) = part.strip_prefix("--hash-style=") {
                    if let Some(h) = parse_hash_style(v) {
                        result.hash_style = h;
                    }
                } else if part == "--hash-style" && j + 1 < parts.len() {
                    j += 1;
                    if let Some(h) = parse_hash_style(parts[j]) {
                        result.hash_style = h;
                    }
                } else if misc_option(&mut result, part) {
                    // consumed
                } else if apply_plain_flag(&mut result, part) {
                    // consumed
                } else if let Some(style) = part.strip_prefix("--build-id=") {
                    result.build_id = !matches!(style, "none" | "0");
                } else if part == "--build-id" {
                    result.build_id = true;
                } else if part == "--no-build-id" {
                    result.build_id = false;
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
        } else if arg == "--as-needed" {
            as_needed = true;
        } else if arg == "--no-as-needed" {
            as_needed = false;
        } else if let Some(v) = arg.strip_prefix("--hash-style=") {
            if let Some(h) = parse_hash_style(v) {
                result.hash_style = h;
            }
        } else if arg == "--hash-style" && i + 1 < args.len() {
            i += 1;
            if let Some(h) = parse_hash_style(args[i]) {
                result.hash_style = h;
            }
        } else if arg == "--threads" && i + 1 < args.len() {
            i += 1;
            // Lenient like GNU: a bad value is ignored, not fatal.  lccc
            // links single-threaded; the value is recorded, not acted on.
            result.threads = args[i].parse().ok();
        } else if misc_option(&mut result, arg) {
            // consumed
        } else if apply_plain_flag(&mut result, arg) {
            // consumed
        } else if let Some(style) = arg.strip_prefix("--build-id=") {
            result.build_id = !matches!(style, "none" | "0");
        } else if arg == "--build-id" {
            result.build_id = true;
        } else if arg == "--no-build-id" {
            result.build_id = false;
        } else if !arg.starts_with('-') && Path::new(arg).exists() {
            result.extra_object_files.push(arg.to_string());
            result.inputs.push(InputItem {
                name: arg.to_string(),
                is_lib: false,
                whole_archive,
                as_needed,
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
        assert_eq!(
            parse_linker_args(&args(&["-Map=out.map"]))
                .map_path
                .as_deref(),
            Some("out.map")
        );
        assert_eq!(
            parse_linker_args(&args(&["-Map", "out.map"]))
                .map_path
                .as_deref(),
            Some("out.map")
        );
        assert_eq!(
            parse_linker_args(&args(&["-Wl,-Map=out.map"]))
                .map_path
                .as_deref(),
            Some("out.map")
        );
        assert_eq!(
            parse_linker_args(&args(&["-Wl,-Map,out.map"]))
                .map_path
                .as_deref(),
            Some("out.map")
        );
    }

    #[test]
    fn dynamic_linker_is_preserved_in_driver_and_ld_spellings() {
        for argv in [
            vec!["--dynamic-linker=/tmp/ld.so"],
            vec!["--dynamic-linker", "/tmp/ld.so"],
            vec!["-Wl,--dynamic-linker=/tmp/ld.so"],
            vec!["-Wl,--dynamic-linker,/tmp/ld.so"],
        ] {
            assert_eq!(
                parse_linker_args(&args(&argv)).dynamic_linker.as_deref(),
                Some("/tmp/ld.so"),
                "argv={argv:?}"
            );
        }
    }

    #[test]
    fn map_path_absent_by_default() {
        assert!(
            parse_linker_args(&args(&["-static", "a.o"]))
                .map_path
                .is_none()
        );
    }

    /// A path containing '=' (legal) must survive intact.
    #[test]
    fn map_path_with_equals_in_filename() {
        assert_eq!(
            parse_linker_args(&args(&["-Map=/tmp/a=b.map"]))
                .map_path
                .as_deref(),
            Some("/tmp/a=b.map")
        );
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
    let Some(paren) = source_name.find('(') else {
        return false;
    };
    if !source_name.ends_with(')') {
        return false;
    }
    let archive = &source_name[..paren];
    let base = archive.rsplit('/').next().unwrap_or(archive);
    exclude
        .iter()
        .any(|e| e.eq_ignore_ascii_case("ALL") || e == base || e == archive)
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
        // Unique per call, not just per process: `cargo test` runs these
        // concurrently in one binary, so a directory keyed only on the PID is
        // shared between tests and the first one to finish deletes the others'
        // files out from under them. That produced a genuinely flaky failure
        // (whole_archive_positional_via_wl passed in isolation, failed in a
        // full run) which is far worse than an outright bug: it trains you to
        // rerun instead of investigate.
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let uniq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let td = std::env::temp_dir().join(format!("lccc_args_{}_{}", std::process::id(), uniq));
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
            let got: Vec<(String, bool)> = r
                .inputs
                .iter()
                .map(|i| {
                    (
                        std::path::Path::new(&i.name)
                            .file_name()
                            .unwrap()
                            .to_string_lossy()
                            .to_string(),
                        i.whole_archive,
                    )
                })
                .collect();
            assert_eq!(
                got,
                vec![
                    ("a.a".to_string(), false),
                    ("b.a".to_string(), true),
                    ("c.a".to_string(), false),
                ]
            );
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
            let r = parse_linker_args(&args(&[&p("first.o"), "-lm", &p("second.o"), "-lpthread"]));
            let got: Vec<(String, bool)> = r
                .inputs
                .iter()
                .map(|i| {
                    (
                        std::path::Path::new(&i.name)
                            .file_name()
                            .unwrap()
                            .to_string_lossy()
                            .to_string(),
                        i.is_lib,
                    )
                })
                .collect();
            assert_eq!(
                got,
                vec![
                    ("first.o".to_string(), false),
                    ("m".to_string(), true),
                    ("second.o".to_string(), false),
                    ("pthread".to_string(), true),
                ]
            );
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

#[cfg(test)]
mod z_isa_level_tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    /// Every `-z x86-64-*` level (binutils
    /// `ld/emulparams/x86-64-level.sh`), spelled the way the driver
    /// forwards it (`-Wl,-z,<kw>`): 0 = unset is the default, baseline =
    /// 1, v2/v3/v4 = 2/3/4.  Last one wins, like the other `-z` toggles.
    #[test]
    fn isa_levels_parse_to_level_numbers() {
        let d = parse_linker_args(&args(&[]));
        assert_eq!(d.z_isa_level, 0, "unset by default");
        assert_eq!(d.z_isa_level_invalid, None);
        for (kw, want) in [
            ("x86-64-baseline", 1),
            ("x86-64-v2", 2),
            ("x86-64-v3", 3),
            ("x86-64-v4", 4),
        ] {
            let r = parse_linker_args(&args(&[&format!("-Wl,-z,{kw}")]));
            assert_eq!(r.z_isa_level, want, "-z {kw}");
            assert_eq!(r.z_isa_level_invalid, None, "-z {kw} must not flag invalid");
        }
        let r = parse_linker_args(&args(&["-Wl,-z,x86-64-v2", "-Wl,-z,x86-64-v4"]));
        assert_eq!(r.z_isa_level, 4, "last level wins");
    }

    /// A bad `x86-64-v` suffix is kept for the link path to reject —
    /// GNU fatals with `invalid x86-64 ISA level`, and silently ignoring
    /// the typo would link an image for the wrong microarchitecture.
    /// `v1` is invalid (baseline covers it); so are a bare `x86-64-v`,
    /// out-of-range versions, and non-numeric suffixes.
    #[test]
    fn isa_level_v_typos_are_kept_for_rejection() {
        for kw in [
            "x86-64-v1",
            "x86-64-v5",
            "x86-64-v9",
            "x86-64-v",
            "x86-64-v2x",
            "x86-64-v3x",
        ] {
            let r = parse_linker_args(&args(&[&format!("-Wl,-z,{kw}")]));
            assert_eq!(r.z_isa_level, 0, "-z {kw} must not set a level");
            assert_eq!(
                r.z_isa_level_invalid.as_deref(),
                Some(kw),
                "-z {kw} must be kept for rejection"
            );
        }
    }

    /// Any OTHER `x86-64-*` keyword is warned about and ignored — GNU's
    /// `-z <kw> ignored` (measured on ld 2.44), not the invalid-level
    /// fatal.  That includes a non-`v` keyword, a bare prefix, and a
    /// near-miss of baseline.  (Wrong-case `X86-64-v3` also warns under
    /// GNU, but it misses the lowercase prefix and lands in the generic
    /// silent-unknown bucket — the pre-existing unknown-`-z` gap, not
    /// this rule; see the follow-up notes.)
    #[test]
    fn isa_level_non_v_keywords_warn_and_ignore() {
        for kw in ["x86-64-avx512", "x86-64-", "x86-64-baselineX"] {
            let r = parse_linker_args(&args(&[&format!("-Wl,-z,{kw}")]));
            assert_eq!(r.z_isa_level, 0, "-z {kw} must not set a level");
            assert_eq!(
                r.z_isa_level_invalid, None,
                "-z {kw} must not be a fatal error"
            );
            assert!(
                r.z_ignored_keywords.iter().any(|k| k == kw),
                "-z {kw} must land on the warn-and-ignore list"
            );
        }
    }

    /// The joined `-z<keyword>` spelling inside a `-Wl,` group behaves
    /// exactly like the split `-Wl,-z,<keyword>` spelling (GNU accepts
    /// both identically, measured on 2.44).
    #[test]
    fn joined_z_keywords_match_split_form() {
        let r = parse_linker_args(&args(&["-Wl,-zrelro"]));
        assert!(r.z_relro);
        let r = parse_linker_args(&args(&["-Wl,-znorelro"]));
        assert!(!r.z_relro);
        let r = parse_linker_args(&args(&["-Wl,-zibt", "-Wl,-zshstk"]));
        assert!(r.z_ibt && r.z_shstk);
        let r = parse_linker_args(&args(&["-Wl,-zx86-64-v3"]));
        assert_eq!(r.z_isa_level, 3);
        assert_eq!(r.z_isa_level_invalid, None);
        let r = parse_linker_args(&args(&["-Wl,-zx86-64-v9"]));
        assert_eq!(r.z_isa_level_invalid.as_deref(), Some("x86-64-v9"));
        let r = parse_linker_args(&args(&["-Wl,-zx86-64-foo"]));
        assert_eq!(r.z_isa_level_invalid, None);
        assert!(r.z_ignored_keywords.iter().any(|k| k == "x86-64-foo"));
    }

    /// `--no-undefined-version` is a real GNU flag ("Disallow undefined
    /// version") so both the plain and the `-Wl,` spelling must parse —
    /// it is recorded, not enforced (see the field docs).
    #[test]
    fn no_undefined_version_parses_but_is_record_only() {
        let r = parse_linker_args(&args(&["--no-undefined-version"]));
        assert!(r.no_undefined_version);
        let r = parse_linker_args(&args(&["-Wl,--no-undefined-version"]));
        assert!(r.no_undefined_version);
        let r = parse_linker_args(&args(&[]));
        assert!(!r.no_undefined_version);
    }

    /// `property_link_flags` maps the parsed `-z` state onto the CET
    /// merge flags, and renders an invalid level with GNU's wording.
    #[test]
    fn property_link_flags_maps_levels_and_rejects_invalid() {
        let r = parse_linker_args(&args(&["-Wl,-z,ibt", "-Wl,-z,shstk", "-Wl,-z,x86-64-v3"]));
        let f = r.property_link_flags().expect("valid flags must convert");
        assert!(f.ibt && f.shstk);
        assert!(!f.lam_u48 && !f.lam_u57);
        assert_eq!(f.isa_level, 3);

        let bad = parse_linker_args(&args(&["-Wl,-z,x86-64-v9"]));
        assert_eq!(
            bad.property_link_flags().unwrap_err(),
            "invalid x86-64 ISA level: x86-64-v9"
        );
        // Deferred rejection is any-invalid-wins: GNU would have fatalled
        // at parse time on the bad keyword no matter what surrounds it.
        let mixed = parse_linker_args(&args(&["-Wl,-z,x86-64-v3", "-Wl,-z,x86-64-v9"]));
        assert!(mixed.property_link_flags().is_err());
        let mixed2 = parse_linker_args(&args(&["-Wl,-z,x86-64-v9", "-Wl,-z,x86-64-v3"]));
        assert!(mixed2.property_link_flags().is_err());
    }
}
