//! `lccc-ld`: GNU-ld-compatible command-line driver for the LCCC linker.
//!
//! Supports the invocation styles used by large build systems that call the
//! linker directly:
//!
//! * Kernel-style script links (`scripts/link-vmlinux.sh`):
//!   `lccc-ld -m elf_x86_64 --script=vmlinux.lds -o vmlinux --whole-archive …`
//!   → script-driven layout engine (`emit_script`).
//! * Relocatable links: `lccc-ld -r a.o b.o -o ab.o` → `emit_rel`.
//! * Standard userspace links, exactly as gcc/clang spawn the system ld:
//!   `lccc-ld -o app crt1.o crti.o app.o crtn.o -Ldir -lc
//!            --dynamic-linker /lib64/ld-linux-x86-64.so.2`
//!   → the same `link_builtin` pipeline the lccc compiler driver uses
//!   (symbol resolution, archives/group semantics, PLT/GOT, RELRO,
//!   eh_frame_hdr, build-id, gc-sections, …).
//!
//! The userspace path makes `lccc-ld` a drop-in for `ld` in Makefiles
//! (`make LD=lccc-ld`) and lets the differential benchmark compare the same
//! CLI against bfd/mold instead of routing through the compiler driver.

/// Expand `@file` response-file arguments (GNU ld convention).
/// Each `@path` argument is replaced by the file's contents split on
/// whitespace (respecting simple single/double quoting and backslash
/// escapes). Non-`@` arguments pass through unchanged. If the file
/// cannot be read, the original `@path` string is preserved so the
/// downstream parser reports it as a missing input (matching GNU ld).
///
/// The kernel's `cmd_ld_multi_m` (scripts/Makefile.build) invokes the
/// linker as `$(LD) $(ld_flags) -r -o $@ @$<` where `$<` is a `.mod`
/// file listing the module's constituent `.o` files. Without this
/// expansion lccc-ld tried to open a file literally named
/// `@arch/.../foo.mod` and failed with ENOENT, breaking every
/// multi-object module link.
fn expand_response_files(args: &[String]) -> Vec<String> {
    let mut result = Vec::new();
    for arg in args {
        if let Some(path) = arg.strip_prefix('@') {
            if let Ok(contents) = std::fs::read_to_string(path) {
                for token in split_response_file(&contents) {
                    result.push(token);
                }
            } else {
                result.push(arg.clone());
            }
        } else {
            result.push(arg.clone());
        }
    }
    result
}

/// Split response-file contents into tokens, handling simple quoting.
fn split_response_file(contents: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_single = false;
    let mut in_double = false;
    let mut escape = false;
    for ch in contents.chars() {
        if escape {
            current.push(ch);
            escape = false;
            continue;
        }
        match ch {
            '\\' if !in_single => escape = true,
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            c if c.is_whitespace() && !in_single && !in_double => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            c => current.push(c),
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

fn main() {
    let raw_args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<String> = expand_response_files(&raw_args);
    if let Some(root) = sysroot_arg(&args) {
        // Publish `--sysroot` without mutating the process environment: glibc's
        // `setenv` may reallocate the `environ` array another thread is walking
        // (why edition 2024 made `set_var` unsafe), and this tree confines
        // environment mutation to `src/test_support.rs`.  The backends resolve
        // the same value through `backend::common::sysroot`.
        lccc::linker_entry::set_sysroot_override(root);
    }

    // Last line of defence against a panic on malformed input.
    //
    // A linker is routinely fed attacker-influenced or simply corrupt object
    // files (truncated build artefacts, interrupted writes, bad archives).
    // GNU ld, mold and wild all answer those with a diagnostic and exit code
    // 1.  Without this guard a Rust panic escapes as exit code 101 plus a
    // backtrace note, which build systems report as an internal toolchain
    // failure and which hides the offending file from the user.
    //
    // The panic hook is replaced so the default "thread panicked at ..."
    // spew is suppressed, and the payload is rendered in GNU style with the
    // location that failed — enough to file a bug, not a stack dump.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if std::env::var_os("LCCC_LD_PANIC_TRACE").is_some() {
            default_hook(info);
        }
    }));

    let result = std::panic::catch_unwind(|| run(&args));

    match result {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            eprintln!("lccc-ld: error: {}", e);
            std::process::exit(1);
        }
        Err(payload) => {
            let msg = payload
                .downcast_ref::<&str>()
                .map(|s| (*s).to_string())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "unknown panic".to_string());
            eprintln!("lccc-ld: internal error: {}", msg);
            eprintln!(
                "lccc-ld: this is a bug; re-run with LCCC_LD_PANIC_TRACE=1 \
                 (and RUST_BACKTRACE=1) for details"
            );
            // Exit 1, like every other linker rejecting an input, so build
            // systems surface the file rather than an internal-error code.
            std::process::exit(1);
        }
    }
}

/// Options that a GNU-compatible linker must accept and that provably do not
/// change the image lccc emits.
///
/// These are not "unknown flags we hope are harmless": each one is either
/// (a) a driver artefact gcc emits on every link, or (b) a diagnostic/
/// bookkeeping switch with no effect on layout, symbols or relocations.
/// Warning about them made `gcc -fuse-ld=lccc` print a dozen lines of noise
/// per invocation, which trains users to ignore lccc's output — the opposite
/// of what a diagnostic is for.
///
/// Anything genuinely unsupported still warns, and anything that could change
/// semantics (`-plugin`) is handled explicitly rather than listed here.
/// True when `path` holds GCC or LLVM LTO bytecode rather than a real object.
///
/// * GCC slim-LTO objects are ELF files whose sections are `.gnu.lto_*`; the
///   ELF header parses fine, which is exactly why this needs an explicit
///   check rather than relying on the parser to fail.
/// * Clang emits raw LLVM bitcode (`BC\xc0\xde`), optionally wrapped.
///
/// Cheap: reads at most the first 4 KiB and never allocates on the hot path.
fn is_lto_bytecode(path: &str) -> bool {
    use std::io::Read;
    let Ok(mut f) = std::fs::File::open(path) else {
        return false;
    };
    let mut head = [0u8; 4096];
    let Ok(n) = f.read(&mut head) else {
        return false;
    };
    let head = &head[..n];
    if head.len() >= 4 {
        // LLVM bitcode magic, and the bitcode-wrapper magic.
        if &head[..4] == b"BC\xc0\xde" || &head[..4] == b"\xde\xc0\x17\x0b" {
            return true;
        }
    }
    // GCC slim LTO: an ELF whose section names begin with .gnu.lto_
    head.windows(9).any(|w| w == b".gnu.lto_")
}

/// Take the value of `--opt VALUE` or `--opt=VALUE`.
///
/// GNU ld accepts both spellings for the options that take an argument, and a
/// linker that only understands one of them rejects valid command lines.
fn two_arg<'a>(a: &'a str, args: &'a [String], i: &mut usize) -> String {
    if let Some((_, v)) = a.split_once('=') {
        return v.to_string();
    }
    *i += 1;
    args.get(*i).cloned().unwrap_or_default()
}

/// Handle one `-z KEYWORD`, shared by the space form (`-z relro`, a match
/// arm below) and the joined form (`-zrelro`, caught in the catch-all
/// chain): GNU ld accepts the two identically. Forward both spellings through
/// the shared linker-argument parser so syntax validation and page-size
/// resolution cannot drift between the driver and backend paths.
fn handle_z_keyword(kw: &str, passthrough: &mut Vec<String>) {
    passthrough.push(format!("-Wl,-z,{kw}"));
}

/// Warn once per distinct unimplemented option.
///
/// Every one of these is accepted by GNU ld and changes the output image or a
/// requested diagnostic, so swallowing it quietly hands the caller a binary
/// that is not what it asked for.  Deduplicating keeps recursive builds from
/// drowning real diagnostics.
/// Derive the GNU property note flags (CET / ISA level) from the
/// passthrough arguments for script links, which never run
/// `parse_linker_args`.  `-z` keywords reach this list joined as
/// `-Wl,-z,<kw>` by the argument loop above.  Returns the flags plus any
/// ignored `=<value>` forms (`-z ibt=func`), which GNU ld accepts and warns
/// about.  An unparseable `-z x86-64-*` level is a hard error, as in GNU ld.
fn passthrough_property_flags(
    passthrough: &[String],
) -> Result<(lccc::linker_entry::PropertyLinkFlags, Vec<String>), String> {
    let mut flags = lccc::linker_entry::PropertyLinkFlags {
        ibt: false,
        shstk: false,
        lam_u48: false,
        lam_u57: false,
        isa_level: 0,
    };
    let mut ignored: Vec<String> = Vec::new();
    for a in passthrough {
        if let Some(kw) = a.strip_prefix("-Wl,-z,") {
            match kw {
                "ibt" => flags.ibt = true,
                "shstk" => flags.shstk = true,
                "lam-u48" => flags.lam_u48 = true,
                "lam-u57" => flags.lam_u57 = true,
                // `-z x86-64-{baseline,v2,v3,v4}` (binutils
                // `ld/emulparams/x86-64-level.sh`), with GNU's rule as in
                // `parse_linker_args` (measured on ld 2.44): a bad
                // `x86-64-v` suffix is the `invalid x86-64 ISA level`
                // fatal, while any OTHER `x86-64-*` keyword is warned
                // about and ignored — never fatal, never silent.
                "x86-64-baseline" => flags.isa_level = 1,
                k if k.starts_with("x86-64-v") => match k["x86-64-v".len()..].parse::<u32>() {
                    Ok(n) if (2..=4).contains(&n) => flags.isa_level = n,
                    _ => return Err(format!("invalid x86-64 ISA level: {k}")),
                },
                k if k.starts_with("x86-64-") => {
                    ignored.push(k.to_string());
                }
                "common-page-size" => ignored.push(kw.to_string()),
                k if k.starts_with("common-page-size=") => {
                    // Script layout currently has no separate common-page
                    // granularity; warn (or fail under --fatal-warnings).
                    ignored.push(k.to_string());
                }
                k if k.starts_with("ibt=") || k.starts_with("shstk=") || k.starts_with("lam-u") => {
                    ignored.push(k.to_string());
                }
                _ => {}
            }
        }
    }
    Ok((flags, ignored))
}

fn warn_unimplemented(a: &str) {
    use std::collections::HashSet;
    use std::sync::OnceLock;
    static SEEN: OnceLock<std::sync::Mutex<HashSet<String>>> = OnceLock::new();
    let seen = SEEN.get_or_init(|| std::sync::Mutex::new(HashSet::new()));
    let key = a.split('=').next().unwrap_or(a).to_string();
    if let Ok(mut guard) = seen.lock()
        && guard.insert(key.clone())
    {
        eprintln!(
            "lccc-ld: warning: '{}' is accepted but not implemented; \
             the requested behavior is not applied",
            key
        );
    }
}

fn is_benign_ignorable(a: &str) -> bool {
    // Diagnostic / bookkeeping switches with no layout effect.
    matches!(
        a,
        "--eh-frame-hdr"          // we always emit .eh_frame_hdr
        | "--no-add-needed"
        | "--no-copy-dt-needed-entries"
        | "--no-warn-mismatch"
        | "--no-warn-search-mismatch"
        // No backend currently emits RWX LOAD-segment warnings. Explicitly
        // disabling that diagnostic is therefore already honored; do not
        // promote a spurious "not implemented" warning under --fatal-warnings.
        // This does NOT implement --warn-rwx-segments, which remains diagnosed.
        | "--no-warn-rwx-segments"
        | "--no-warn-execstack"
        | "--warn-execstack"
        // GNU's `--warn-textrel` turns the text-relocation check to "warn",
        // which is already lccc-ld's default (`TextrelPolicy::Warn`).
        | "--warn-textrel"
        | "--disable-linker-version"
        | "--no-relax"
        // Driver-level flags that reach a standalone linker when a build system
        // hands the same list to both. A linker never adds default libraries on
        // its own -- that is the driver's job; for ld, -nostdlib only drops the
        // built-in library search path (read where that path is chosen), and
        // warning about any of these on every kernel-style link is noise that
        // hides the warnings that matter.
        | "-nostdlib" | "-nostartfiles" | "-nodefaultlibs"
        | "-O0" | "-O1" | "-O2" | "-O3" // ld's own -O is a size/speed hint
    ) || a.starts_with("-plugin-opt=")
}

/// GNU ld's `--sysroot=DIR` (the last one wins), unless it is the host root:
/// `--sysroot=/`, which cross drivers pass, changes no path.
fn sysroot_arg(args: &[String]) -> Option<&str> {
    let mut root = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if let Some(dir) = a.strip_prefix("--sysroot=") {
            root = Some(dir);
        } else if a == "--sysroot" {
            root = it.next().map(String::as_str);
        }
    }
    root.filter(|dir| !dir.trim_end_matches('/').is_empty())
}

/// The emulation GNU ld's `-m NAME` selects.
fn parse_emulation(name: &str) -> Result<Emulation, String> {
    match name {
        "elf_x86_64" | "elf64_x86_64" => Ok(Emulation::X86_64),
        "elf_i386" | "i386linux" => Ok(Emulation::I386),
        "aarch64linux" | "aarch64elf" | "aarch64_elf64_le_vec" => Ok(Emulation::Aarch64),
        "elf64lriscv" | "elf64lriscv_lp64d" | "elf64lriscv_lp64" => Ok(Emulation::Riscv64),
        other => Err(format!("unsupported emulation '-m {other}'")),
    }
}

/// `passthrough` with the positional input files re-inserted where they
/// appeared (see `positional_files` in `run`): the elf_i386 userspace argument
/// list, and the order `-r`/`-T` links expand `-l` in.
fn ordered_args(passthrough: &[String], positional: &[(usize, String)]) -> Vec<String> {
    let mut out = Vec::with_capacity(passthrough.len() + positional.len());
    let mut pending = positional.iter().peekable();
    for (idx, tok) in passthrough.iter().enumerate() {
        while let Some((_, t)) = pending.next_if(|(at, _)| *at == idx) {
            out.push(t.clone());
        }
        out.push(tok.clone());
    }
    out.extend(pending.map(|(_, t)| t.clone()));
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Emulation {
    X86_64,
    I386,
    Aarch64,
    Riscv64,
}

fn run(args: &[String]) -> Result<(), String> {
    let mut output = "a.out".to_string();
    let mut script_path: Option<String> = None;
    let mut inputs: Vec<(String, bool)> = Vec::new(); // (path, whole_archive)
    let mut whole_archive = false;
    let mut emit_symtab = true;
    let mut strip_debug = false;
    let mut relocatable = false;
    let mut emit_relocs = false;
    let mut gc_sections = false;
    let mut is_pie = false;
    let mut is_static = false;
    let mut build_id_style = lccc::linker_entry::BuildIdStyle::None;
    let mut entry_override: Option<String> = None;
    let mut shared = false;
    let mut soname: Option<String> = None;
    let mut dt_symbolic = false;
    let mut emulation = Emulation::X86_64;
    let mut trace = false;
    let mut fatal_warnings = false;
    let mut print_memory_usage = false;
    let mut oformat_binary = false;
    let mut top_warnings: Vec<String> = Vec::new();
    // Options accepted but not implemented are diagnosed after parsing so
    // --fatal-warnings applies regardless of their position in argv.
    let mut unimplemented_warnings: Vec<String> = Vec::new();
    // Arguments forwarded verbatim into the builtin userspace pipeline
    // (parse_linker_args understands the GNU spellings directly).
    let mut passthrough: Vec<String> = Vec::new();
    // --defsym definitions, kept separately from `passthrough` because the
    // script-driven link never re-parses passthrough arguments.
    let mut defsyms: Vec<(String, String)> = Vec::new();
    // `-u SYM` / `--undefined=SYM`: force-undefined names that pull archive
    // members. Script links (`-T`) never consult `passthrough`, so these must
    // be collected separately and handed to `load_inputs_x86`.
    let mut undefined_symbols: Vec<String> = Vec::new();
    // Set when gcc handed us the LTO plugin; see the -plugin arm below.
    let mut saw_lto_plugin = false;
    // Positional input files, tagged with the `passthrough` length at the
    // time they appeared, so they keep their command-line position relative
    // to the `-l`, `-Bstatic` and `--whole-archive` tokens there.  The i386
    // userspace pipeline and the `-r`/`-T` modes resolve one ordered input
    // list (archive extraction and library search depend on position); the
    // x86-64 userspace path keeps consuming `inputs` + `passthrough`
    // separately.  See `ordered_args`.
    let mut positional_files: Vec<(usize, String)> = Vec::new();

    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "-o" => {
                i += 1;
                output = args.get(i).cloned().ok_or("-o needs an argument")?;
            }
            // Emulation selects both the input parser and output ELF
            // class/architecture.  GNU ld takes it separate or attached:
            // gcc passes `-m elf_x86_64`, the AArch64 and RISC-V cross
            // drivers `-maarch64linux` / `-melf64lriscv`.
            "-m" => {
                i += 1;
                let name = args.get(i).ok_or("-m needs an argument")?;
                emulation = parse_emulation(name)?;
            }
            a if a.starts_with("-m") && a.len() > 2 => emulation = parse_emulation(&a[2..])?,
            // Read before anything runs (`sysroot_arg`).
            "--sysroot" => i += 1,
            a if a.starts_with("--sysroot=") => {}
            // Every emulation above is little-endian.
            "-EL" => {}
            "-EB" => return Err("big-endian output is not supported".into()),
            "-t" | "--trace" => {
                trace = true;
                passthrough.push("-t".to_string());
            }
            // GNU ld's COMMON-symbol diagnostics, implemented by every
            // backend (`linker_common::warn_common`).
            "--warn-common" => passthrough.push("--warn-common".to_string()),
            "--fatal-warnings" => {
                fatal_warnings = true;
                passthrough.push("--fatal-warnings".to_string());
            }
            "--no-fatal-warnings" => {
                fatal_warnings = false;
                passthrough.push("--no-fatal-warnings".to_string());
            }
            "--print-memory-usage" => {
                print_memory_usage = true;
                passthrough.push("--print-memory-usage".to_string());
            }
            "-T" | "--script" => {
                i += 1;
                script_path = Some(args.get(i).cloned().ok_or("-T needs an argument")?);
            }
            "-r" | "--relocatable" | "-i" => relocatable = true,
            // --emit-relocs: keep the applied relocations in the output.
            // The kernel's arch/x86/tools/relocs pass consumes them to build
            // the KASLR relocation table; ignoring the flag produced a kernel
            // that linked cleanly and then failed to boot.
            "--emit-relocs" | "-q" => {
                // Recorded for the `-T` script path (which implements
                // retention) AND forwarded so the built-in/shared paths warn
                // honestly (their emitters cannot retain — swallowing the
                // flag here used to drop it silently on exactly those paths).
                emit_relocs = true;
                passthrough.push("--emit-relocs".to_string());
            }
            // Forwarded so `parse_linker_args` records them: --no-emit-relocs
            // resets, --threads/--no-threads are deterministic hints (lccc
            // links single-threaded; the value is recorded, not acted on).
            // (There is deliberately no --isa-level / -march forwarding:
            // GNU ld has no such options — the ISA level is `-z
            // x86-64-{baseline,v2,v3,v4}` — so those spellings fall through
            // to the unknown-option warning below instead of being blessed
            // here.  --no-undefined-version IS a real GNU flag, so it gets
            // the accepted-but-not-implemented warning in the general flag
            // dispatch instead — falling through to "unknown option" would
            // be factually wrong.)
            "--no-emit-relocs" | "--no-threads" | "--threads" => passthrough.push(a.to_string()),
            a if a.starts_with("--threads=") => {
                passthrough.push(a.to_string());
            }
            // Forwarded as well as recorded: `is_pie` drives the local
            // decisions below, while the copy in `passthrough` is what
            // `parse_linker_args` reads to make the emitter produce ET_DYN.
            "-pie" | "--pic-executable" => {
                is_pie = true;
                passthrough.push("--pic-executable".to_string());
            }
            "-no-pie" => {
                is_pie = false;
                passthrough.push("--no-pic-executable".to_string());
            }
            // DT_RPATH vs DT_RUNPATH for -rpath.  Only the `-Wl,` spelling
            // reaches the shared parser, so a driver calling this as `ld`
            // (gcc passes them bare) used to lose both to the unknown-option
            // warning, and a plain -rpath always became DT_RUNPATH.
            "--disable-new-dtags" | "--enable-new-dtags" => {
                passthrough.push(format!("-Wl,{a}"));
            }
            "-shared" | "-Bshareable" => shared = true,
            "--no-dynamic-linker" | "--no-ld-generated-unwind-info" => {}
            // --whole-archive is POSITIONAL: it applies to the archives that
            // follow it.  The local flag covers positional inputs; the shared
            // argument parser needs the flag too, because `-l` libraries are
            // resolved there and would otherwise be loaded selectively -- the
            // link succeeded while silently dropping the unreferenced members
            // the user asked to keep (constructors, plugin registration).
            "--whole-archive" => {
                whole_archive = true;
                passthrough.push("--whole-archive".to_string());
            }
            "--no-whole-archive" => {
                whole_archive = false;
                passthrough.push("--no-whole-archive".to_string());
            }
            "--start-group" | "--end-group" | "-(" | "-)" => {
                // Builtin archive loading already iterates to a fixpoint
                // (global group semantics), which subsumes group regions.
            }
            // Forwarded like `-s` below: the built-in emitters read it out
            // of `parse_linker_args`.
            "--strip-debug" | "-S" => {
                strip_debug = true;
                passthrough.push("--strip-debug".to_string());
            }
            // Recorded for the linker-script path AND forwarded: the built-in
            // emitter reads `strip_all` out of `parse_linker_args`, so without
            // the passthrough copy `-s` silently produced a fully-symbolled
            // binary.  That is worse than ignoring the flag, because the user
            // believes they shipped a stripped image.
            "--strip-all" | "-s" => {
                emit_symtab = false;
                passthrough.push("--strip-all".to_string());
            }
            "-v" | "-V" | "--version" => {
                println!("{}", lccc::linker_entry::GNU_LD_VERSION_OUTPUT);
                return Ok(());
            }
            "--help" => {
                println!("Usage: lccc-ld [options] file...");
                println!(
                    "  Standard userspace (elf_x86_64, elf_i386), -r relocatable (elf_x86_64),"
                );
                println!("  and -T script links (both) supported.");
                return Ok(());
            }
            "-e" | "--entry" => {
                i += 1;
                entry_override = args.get(i).cloned();
                if let Some(e) = &entry_override {
                    passthrough.push(format!("--entry={}", e));
                }
            }
            // GNU ld spells "write the map to stdout" as --print-map / -M.
            // These were previously swallowed by the ignore list, so the user
            // got a silent no-op instead of a map.
            "--print-map" | "-M" => passthrough.push("-Map=-".to_string()),
            "-Map" => {
                // Two-argument form: `-Map FILE`. Re-spell as the joined form
                // so the shared parser handles both identically.
                i += 1;
                if let Some(v) = args.get(i) {
                    passthrough.push(format!("-Map={}", v));
                }
            }
            a if a.starts_with("--dynamic-linker=") || a.starts_with("-dynamic-linker=") => {
                // Preserve a caller-selected interpreter all the way to the
                // ELF emitter.  Previously this option was accepted but
                // discarded, so staging links silently ran with the host
                // loader and could bind against the wrong libc.
                let p = a.split_once('=').map(|(_, value)| value).unwrap_or("");
                if p.is_empty() {
                    return Err("--dynamic-linker needs a non-empty path".into());
                }
                passthrough.push(format!("--dynamic-linker={p}"));
            }
            "--dynamic-linker" | "-dynamic-linker" | "-I" => {
                i += 1;
                let p = args.get(i).ok_or("--dynamic-linker needs an argument")?;
                if p.is_empty() {
                    return Err("--dynamic-linker needs a non-empty path".into());
                }
                passthrough.push(format!("--dynamic-linker={p}"));
            }
            "-z" => {
                i += 1;
                if let Some(kw) = args.get(i) {
                    handle_z_keyword(kw, &mut passthrough);
                }
            }
            "-static" => {
                is_static = true;
                passthrough.push("-static".to_string());
            }
            // GNU ld: `-Bstatic` is positional — it restricts the `-l`
            // searches that follow to `libNAME.a` until `-Bdynamic`, and
            // does not by itself make the output a static executable
            // (`-Wl,-Bstatic -lfoo -Wl,-Bdynamic` links libfoo.a into an
            // otherwise dynamic program).  Both pipelines read the token
            // from its position in `passthrough`.
            "-Bstatic" | "-dn" | "-non_shared" | "-Bdynamic" | "-dy" | "-call_shared" => {
                passthrough.push(a.to_string());
            }
            // gcc's specs save/restore the positional state around groups
            // (`--push-state --as-needed -lgcc_s --pop-state`); dropping the
            // pair leaked `--as-needed` into every later library.
            "--push-state" | "--pop-state" => passthrough.push(a.to_string()),
            "--gc-sections" => {
                gc_sections = true;
                passthrough.push("-Wl,--gc-sections".to_string());
            }
            "--no-gc-sections" => {
                gc_sections = false;
                passthrough.push("-Wl,--no-gc-sections".to_string());
            }
            "--no-undefined" => {
                // Script links resolve every non-weak relocation eagerly.
                passthrough.push(a.to_string());
            }
            "--undefined" => {
                i += 1;
                if let Some(sym) = args.get(i) {
                    undefined_symbols.push(sym.clone());
                    passthrough.push(format!("-Wl,-u,{}", sym));
                }
            }
            a if a.starts_with("--undefined=") => {
                let sym = &a["--undefined=".len()..];
                if !sym.is_empty() {
                    undefined_symbols.push(sym.to_string());
                    passthrough.push(format!("-Wl,-u,{}", sym));
                }
            }
            // Linker-script links bind every reference statically, so the
            // only effect left is the DT_SYMBOLIC/DF_SYMBOLIC tag, which
            // belongs to -Bsymbolic alone: under -Bsymbolic-functions ld.so
            // must still look data up globally.  Last spelling wins (GNU).
            "-Bsymbolic" | "-Bsymbolic-functions" | "-Bno-symbolic" => {
                dt_symbolic = a == "-Bsymbolic";
                passthrough.push(a.to_string());
            }
            // GNU ld accepts BOTH spellings and gcc's driver emits the
            // single-dash one (`gcc -rdynamic` -> `collect2 ... -export-dynamic`).
            // Matching only the double-dash form silently dropped the flag, so
            // `gcc -rdynamic` produced an executable exporting nothing: dlopen'd
            // plugins could not resolve back into the host, and backtrace_symbols
            // lost every name. lccc-ld invoked directly worked, which is what
            // made the bug survive — the test used the direct form.
            "--export-dynamic" | "-export-dynamic" | "-E" => {
                passthrough.push("-rdynamic".to_string())
            }
            "--no-export-dynamic" | "-no-export-dynamic" => {}
            // --as-needed / --no-as-needed are POSITIONAL: they scope the
            // inputs that follow. Forward them so the shared parser can record
            // the state per input, instead of dropping them here (which made
            // every library as-needed and silently discarded a DT_NEEDED the
            // user asked for with --no-as-needed).
            "--as-needed" | "--no-as-needed" => passthrough.push(a.to_string()),
            // COMMON sorting (implemented in the shared allocator): forward
            // every spelling so the shared parser sees it.  Without this arm
            // the flag died here with an "unknown option" warning.
            a if a == "--sort-common"
                || a.starts_with("--sort-common=")
                || a == "--no-sort-common" =>
            {
                passthrough.push(a.to_string())
            }
            "--eh-frame-hdr"
            | "--fix-cortex-a53-843419"
            | "--no-copy-dt-needed-entries"
            | "--allow-shlib-undefined"
            | "-X"
            | "-x" => {}
            _ => {
                if let Some(v) = a.strip_prefix("--script=") {
                    script_path = Some(v.to_string());
                } else if let Some(v) = a.strip_prefix("-T") {
                    if !v.is_empty() {
                        script_path = Some(v.to_string());
                    }
                } else if let Some(v) = a.strip_prefix("--entry=") {
                    entry_override = Some(v.to_string());
                    passthrough.push(a.to_string());
                } else if a.starts_with("-Map=") {
                    // Forwarded verbatim; parse_linker_args understands it and
                    // emit_exec writes the map after address assignment.
                    passthrough.push(a.to_string());
                } else if a.starts_with("--dynamic-linker=") {
                    // handled above for the two-arg form; same policy here
                } else if a.starts_with("-L") {
                    // -Ldir or -L dir
                    if a == "-L" {
                        i += 1;
                        if let Some(d) = args.get(i) {
                            passthrough.push(format!("-L{}", d));
                        }
                    } else {
                        passthrough.push(a.to_string());
                    }
                } else if a.starts_with("-l") {
                    if a == "-l" {
                        i += 1;
                        if let Some(l) = args.get(i) {
                            passthrough.push(format!("-l{}", l));
                        }
                    } else {
                        passthrough.push(a.to_string());
                    }
                } else if a.starts_with("--wrap") {
                    // GNU ld accepts both --wrap=SYM and --wrap SYM.  Only the
                    // joined form was handled, so `ld --wrap foo` fell through
                    // to the unknown-option warning and the link then failed on
                    // the undefined __wrap_foo.
                    let val = two_arg(a, args, &mut i);
                    if !val.is_empty() {
                        passthrough.push(format!("-Wl,--wrap={}", val));
                    }
                } else if a.starts_with("--defsym") {
                    // GNU ld accepts --defsym=SYM=EXPR and --defsym SYM=EXPR.
                    // `two_arg` normalises both; the local `defsyms` copy feeds
                    // the builtin link path (the passthrough copy feeds the
                    // shared-argument parser, which re-classifies via
                    // linker_common::defsym).  A missing `=` is an error rather
                    // than a warning+continue: the next token would otherwise
                    // be consumed as an input filename and the real problem
                    // would scroll away as an unknown-option warning.
                    let val = two_arg(a, args, &mut i);
                    let (name, expr) = val
                        .split_once('=')
                        .ok_or_else(|| format!("--defsym needs SYMBOL=EXPRESSION, got '{val}'"))?;
                    if !name.is_empty() {
                        passthrough.push(format!("-Wl,--defsym={val}"));
                        defsyms.push((name.to_string(), expr.to_string()));
                    }
                } else if a.starts_with("--icf") {
                    // --icf=none|safe|all.  Identical Code Folding is
                    // implemented in the shared pipeline; the driver simply did
                    // not forward it, so `ld --icf=all` was a silent no-op.
                    let val = two_arg(a, args, &mut i);
                    let mode = if val.is_empty() {
                        "all".to_string()
                    } else {
                        val
                    };
                    passthrough.push(format!("-Wl,--icf={}", mode));
                } else if let Some(rest) = a.strip_prefix("-u") {
                    let sym = if rest.is_empty() {
                        i += 1;
                        args.get(i).cloned().unwrap_or_default()
                    } else {
                        rest.to_string()
                    };
                    if !sym.is_empty() {
                        undefined_symbols.push(sym.clone());
                        passthrough.push(format!("-Wl,-u,{}", sym));
                    }
                } else if a.starts_with("-rpath-link") || a.starts_with("--rpath-link") {
                    // Search path for the DT_NEEDED dependencies of input
                    // shared objects.  lccc-ld never loads indirect
                    // dependencies (no --copy-dt-needed-entries, no
                    // shared-object undefined check), so it cannot change
                    // the output; consume the value so it is not mistaken
                    // for an input file.
                    let _ = two_arg(a, args, &mut i);
                } else if let Some(rest) = a.strip_prefix("-rpath=") {
                    passthrough.push(format!("-Wl,-rpath={}", rest));
                } else if a == "-rpath" {
                    i += 1;
                    if let Some(p) = args.get(i) {
                        passthrough.push(format!("-Wl,-rpath={}", p));
                    }
                } else if a.starts_with("-soname") || a.starts_with("--soname") {
                    let val = if let Some(eq) = a.split_once('=') {
                        eq.1.to_string()
                    } else {
                        i += 1;
                        args.get(i).cloned().unwrap_or_default()
                    };
                    if !val.is_empty() {
                        soname = Some(val.clone());
                        passthrough.push(format!("-Wl,-soname,{}", val));
                    }
                } else if a.starts_with("--build-id") {
                    let style_opt = a.split_once('=').map(|(_, v)| v);
                    build_id_style = lccc::linker_entry::BuildIdStyle::parse(true, style_opt)?;
                    passthrough.push(if let Some((_, v)) = a.split_once('=') {
                        format!("-Wl,--build-id={}", v)
                    } else {
                        "-Wl,--build-id".to_string()
                    });
                } else if a.starts_with("--dynamic-list") {
                    let val = two_arg(a, args, &mut i);
                    if val.is_empty() {
                        return Err("--dynamic-list needs a file path".into());
                    }
                    passthrough.push(format!("-Wl,--dynamic-list={val}"));
                } else if a.starts_with("--image-base") {
                    let val = two_arg(a, args, &mut i);
                    if val.is_empty() {
                        return Err("--image-base needs an address".into());
                    }
                    passthrough.push(format!("-Wl,--image-base={val}"));
                } else if a.starts_with("--pack-dyn-relocs") {
                    let val = two_arg(a, args, &mut i);
                    if val.is_empty() {
                        return Err("--pack-dyn-relocs needs a value".into());
                    }
                    passthrough.push(format!("-Wl,--pack-dyn-relocs={val}"));
                } else if a.starts_with("--oformat") || a == "-b" {
                    let val = two_arg(a, args, &mut i);
                    match val.as_str() {
                        "binary" => {
                            oformat_binary = true;
                            passthrough.push("-Wl,--oformat=binary".to_string());
                        }
                        v if v == "default" || v.starts_with("elf") => {
                            oformat_binary = false;
                        }
                        other => {
                            return Err(format!("unsupported --oformat='{other}'"));
                        }
                    }
                } else if a.starts_with("--exclude-libs") {
                    // Forward to the shared pipeline, normalising the
                    // two-argument form to the joined one.
                    let val = if let Some((_, v)) = a.split_once('=') {
                        v.to_string()
                    } else {
                        i += 1;
                        args.get(i).cloned().unwrap_or_default()
                    };
                    if !val.is_empty() {
                        passthrough.push(format!("-Wl,--exclude-libs={}", val));
                    }
                } else if a.starts_with("--version-script") {
                    let val = if let Some(eq) = a.split_once('=') {
                        eq.1.to_string()
                    } else {
                        i += 1;
                        args.get(i).cloned().unwrap_or_default()
                    };
                    if !val.is_empty() {
                        passthrough.push(format!("-Wl,--version-script={}", val));
                    }
                } else if a.starts_with("--hash-style") || a.starts_with("-hash-style") {
                    // Implemented: `gnu`, `sysv` and `both` all produce the
                    // matching `.gnu.hash` / `.hash` sections and DT tags.
                    // GNU ld takes the single-dash spelling too, which the
                    // RISC-V gcc driver passes (`-hash-style=gnu`).
                    // Forwarded so `parse_linker_args` sees it; this used to be
                    // swallowed with a "not implemented" warning, which made
                    // `--hash-style` a no-op no matter what the emitter could do.
                    if let Some((_, v)) = a.split_once('=') {
                        if !v.is_empty() {
                            passthrough.push(format!("--hash-style={v}"));
                        }
                    } else {
                        i += 1;
                        if let Some(v) = args.get(i) {
                            passthrough.push(format!("--hash-style={v}"));
                        }
                    }
                } else if let Some(kw) = a.strip_prefix("-z") {
                    // Joined form `-z<kw>`: GNU ld accepts it exactly like
                    // `-z <kw>`, so it funnels through the same handler as
                    // the space-form match arm rather than warning that
                    // `-z` is unimplemented.  (A bare `-z` never reaches
                    // here — the match arm above consumes it — but an empty
                    // keyword is harmlessly skipped rather than asserted on.)
                    if !kw.is_empty() {
                        handle_z_keyword(kw, &mut passthrough);
                    }
                } else if a.starts_with("--orphan-handling")
                    || a.starts_with("--sort-section")
                    || a.starts_with("--print-")
                    // `--no-undefined-version` is a real GNU flag ("Disallow
                    // undefined version"), so it must not fall through to the
                    // unknown-option warning — but lccc does not check version
                    // references, so accepting it silently would hide the
                    // missing strictness.  Warn once, like the other
                    // accepted-but-not-implemented diagnostics above.
                    || a == "--no-undefined-version"
                {
                    // Accepted but NOT implemented.  These change the image
                    // (or a diagnostic the user asked for), so ignoring them
                    // silently is worse than saying so: the caller believes it
                    // got SysV+GNU hash tables, sorted sections, or a
                    // --print-gc-sections listing and got none of them.  Warn
                    // once per distinct option rather than per invocation so a
                    // recursive make does not scroll them off the screen.
                    unimplemented_warnings.push(a.to_string());
                } else if is_benign_ignorable(a) {
                    // Options every GNU-compatible linker accepts and that do
                    // not change the image we produce.
                } else if a == "-plugin" || a.starts_with("-plugin-opt") {
                    if a == "-plugin" {
                        i += 1;
                    } // skip the plugin path
                    saw_lto_plugin = true;
                } else if a.starts_with('-') {
                    // Unknown flag: warn (or error under --fatal-warnings).
                    top_warnings.push(format!("ignoring unknown option '{a}'"));
                } else {
                    inputs.push((a.to_string(), whole_archive));
                    positional_files.push((passthrough.len(), a.to_string()));
                }
            }
        }
        i += 1;
    }

    if fatal_warnings && !top_warnings.is_empty() {
        return Err(format!("warning treated as error: {}", top_warnings[0]));
    }
    if fatal_warnings && !unimplemented_warnings.is_empty() {
        return Err(format!(
            "warning treated as error: '{}' is not implemented",
            unimplemented_warnings[0]
        ));
    }
    for w in &top_warnings {
        eprintln!("lccc-ld: warning: {w}");
    }
    for option in &unimplemented_warnings {
        warn_unimplemented(option);
    }

    if inputs.is_empty() {
        return Err("no input files".into());
    }
    let elf_i386 = emulation == Emulation::I386;

    // ------------------------------------------------------------------
    // Mode 1: relocatable link (ld -r).
    // ------------------------------------------------------------------
    if relocatable {
        if emulation != Emulation::X86_64 {
            return Err(
                "relocatable (-r) output is currently implemented for elf_x86_64 only".into(),
            );
        }
        if script_path.is_some() {
            if fatal_warnings {
                return Err(
                    "warning treated as error: -r with a linker script: script ignored".into(),
                );
            }
            eprintln!("lccc-ld: warning: -r with a linker script: script ignored");
        }
        let ordered = ordered_args(&passthrough, &positional_files);
        let inputs = lccc::linker_entry::expand_file_mode_libs(&ordered, &inputs, &[], "-r")?;
        let lib_dirs = lccc::backend::linker_common::parse_linker_args(&ordered).extra_lib_paths;
        let mut objects = Vec::new();
        lccc::linker_entry::load_inputs_x86(&inputs, &mut objects, &undefined_symbols, &lib_dirs)?;
        if trace {
            for obj in &objects {
                println!("{}", obj.source_name);
            }
        }
        if strip_debug || !emit_symtab {
            lccc::linker_entry::strip_debug_sections(&mut objects);
        }
        return lccc::linker_entry::link_relocatable_x86(&objects, &output);
    }

    if saw_lto_plugin
        && let Some(bad) = inputs
            .iter()
            .find_map(|(p, _)| is_lto_bytecode(p).then(|| p.clone()))
    {
        return Err(format!(
            "'{}' is LTO bytecode, which requires a linker plugin that \
             lccc-ld does not implement; rebuild that input without -flto \
             (or link it with the compiler driver)",
            bad
        ));
    }

    // ------------------------------------------------------------------
    // Mode 2: script-driven link (kernel-style -T).
    // ------------------------------------------------------------------
    if let Some(script_path) = script_path {
        if !matches!(emulation, Emulation::X86_64 | Emulation::I386) {
            return Err(
                "linker-script (-T) mode is currently implemented for elf_x86_64 and elf_i386"
                    .into(),
            );
        }
        let ordered = ordered_args(&passthrough, &positional_files);
        let script_args = lccc::backend::linker_common::parse_linker_args(&ordered);
        // Resolve command-line errors before reading the script or opening any
        // input. A missing file must not hide an invalid linker option.
        script_args.validate_common_options()?;
        // GNU ld 2.47 uses 4 KiB by default for both x86-64 and i386
        // script links; 2 MiB is an explicit max-page-size request (as used
        // by the Linux kernel). A zero spelling selects that same default.
        let default_page_size = 0x1000;
        let max_page_size = script_args.resolved_page_size(default_page_size)?;
        let (cet_flags, _property_warnings) = passthrough_property_flags(&passthrough)?;
        let mut z_warnings: Vec<String> = script_args
            .z_ignored_keywords
            .iter()
            .map(|kw| format!("-z {kw} ignored"))
            .collect();
        z_warnings.extend(script_args.z_other_keyword_warnings());
        let mut script_option_warnings = Vec::new();
        if script_args.image_base.is_some() {
            script_option_warnings.push(
                "--image-base ignored: explicit image bases are not implemented for linker-script links"
                    .to_string(),
            );
        }
        if script_args.z_pack_relative_relocs {
            script_option_warnings.push(
                "--pack-dyn-relocs=relr / -z pack-relative-relocs ignored: RELR packing is not implemented for linker-script links"
                    .to_string(),
            );
        }
        if script_args.warn_common {
            script_option_warnings.push(
                "--warn-common ignored: common-symbol diagnostics are not implemented for linker-script links"
                    .to_string(),
            );
        }
        if fatal_warnings && !script_option_warnings.is_empty() {
            return Err(format!(
                "warning treated as error: {}",
                script_option_warnings[0]
            ));
        }
        for warning in &script_option_warnings {
            eprintln!("lccc-ld: warning: {warning}");
        }
        if fatal_warnings && !z_warnings.is_empty() {
            return Err(format!("warning treated as error: {}", z_warnings[0]));
        }
        for warning in &z_warnings {
            eprintln!("lccc-ld: warning: {warning}");
        }

        let mut script_src = std::fs::read_to_string(&script_path)
            .map_err(|e| format!("cannot read script '{}': {}", script_path, e))?;
        let search_dirs = lccc::linker_entry::script_search_dirs(&script_src)?;
        let inputs =
            lccc::linker_entry::expand_file_mode_libs(&ordered, &inputs, &search_dirs, "-T")?;
        let mut lib_dirs = script_args.extra_lib_paths;
        lib_dirs.extend(search_dirs.iter().cloned());
        let mut objects = if elf_i386 {
            lccc::linker_entry::load_inputs_i386_script(&inputs, &undefined_symbols)?
        } else {
            let mut objects = Vec::new();
            lccc::linker_entry::load_inputs_x86(
                &inputs,
                &mut objects,
                &undefined_symbols,
                &lib_dirs,
            )?;
            objects
        };
        if trace {
            for obj in &objects {
                println!("{}", obj.source_name);
            }
        }
        if strip_debug || !emit_symtab {
            lccc::linker_entry::strip_debug_sections(&mut objects);
        }
        let real_inputs = objects.len();
        if build_id_style.is_enabled() {
            lccc::linker_entry::append_build_id_object_styled(&mut objects, &build_id_style);
        }
        if let Some(carrier) = lccc::linker_entry::merge_property_into_objects(
            &mut objects,
            real_inputs,
            &cet_flags,
            elf_i386,
        )? {
            objects.push(carrier);
        }
        if let Some(e) = entry_override {
            script_src = format!("ENTRY({})\n{}", e, script_src);
        }
        let extras = lccc::linker_entry::ScriptLinkExtras {
            build_id_style,
            print_memory_usage,
        };
        let res = if elf_i386 {
            lccc::linker_entry::link_with_script_i386_extras(
                &mut objects,
                &script_src,
                &output,
                emit_symtab,
                is_pie || shared,
                emit_relocs,
                gc_sections,
                soname.as_deref(),
                dt_symbolic,
                max_page_size,
                &defsyms,
                &extras,
            )
        } else {
            lccc::linker_entry::link_with_script_x86_extras(
                &mut objects,
                &script_src,
                &output,
                emit_symtab,
                is_pie || shared,
                emit_relocs,
                gc_sections,
                soname.as_deref(),
                dt_symbolic,
                max_page_size,
                &defsyms,
                &extras,
            )
        };
        res?;
        if oformat_binary {
            lccc::linker_entry::convert_elf_file_to_flat_binary(&output)?;
        }
        return Ok(());
    }

    // ------------------------------------------------------------------
    // Mode 3: standard userspace link across x86-64, i386, AArch64, RISC-V.
    // ------------------------------------------------------------------
    if is_pie && is_static {
        return Err("-static-pie is not implemented: lccc-ld cannot yet emit a \
             position-independent static executable"
            .to_string());
    }
    let default_dirs: &[&str] = if args.iter().any(|a| a == "-nostdlib") {
        &[]
    } else {
        match emulation {
            Emulation::X86_64 => lccc::linker_entry::DEFAULT_SEARCH_DIRS_X86_64,
            Emulation::I386 => lccc::linker_entry::DEFAULT_SEARCH_DIRS_I386,
            Emulation::Aarch64 => lccc::linker_entry::DEFAULT_SEARCH_DIRS_AARCH64,
            Emulation::Riscv64 => lccc::linker_entry::DEFAULT_SEARCH_DIRS_RISCV64,
        }
    };
    if emulation == Emulation::I386 {
        let i386_args = ordered_args(&passthrough, &positional_files);
        let res = if shared {
            lccc::linker_entry::link_shared_i386(&output, &i386_args, default_dirs)
        } else {
            lccc::linker_entry::link_builtin_i386(&output, &i386_args, default_dirs)
        };
        res?;
        if oformat_binary {
            lccc::linker_entry::convert_elf_file_to_flat_binary(&output)?;
        }
        return Ok(());
    }
    // GNU ld scopes `--as-needed` positionally; the backends load the files
    // named here in command-line order, so each takes the state it was
    // named under along (`inputs` and `positional_files` are pushed
    // together, one entry per file).
    let positions: Vec<usize> = positional_files.iter().map(|(at, _)| *at).collect();
    let states = lccc::linker_entry::as_needed_states(&passthrough, &positions);
    let mut object_files: Vec<String> = Vec::new();
    let mut object_as_needed: Vec<bool> = Vec::new();
    for ((path, wa), as_needed) in inputs.iter().zip(states) {
        if *wa && path.ends_with(".a") {
            passthrough.push("--whole-archive".to_string());
            passthrough.push(path.clone());
            passthrough.push("--no-whole-archive".to_string());
        } else {
            object_files.push(path.clone());
            object_as_needed.push(as_needed);
        }
    }
    let object_refs: Vec<&str> = object_files.iter().map(|s| s.as_str()).collect();

    let res = match (emulation, shared) {
        (Emulation::X86_64 | Emulation::I386, true) => lccc::linker_entry::link_shared_x86(
            &object_refs,
            &object_as_needed,
            &output,
            &passthrough,
            default_dirs,
        ),
        (Emulation::X86_64 | Emulation::I386, false) => lccc::linker_entry::link_builtin_x86(
            &object_refs,
            &object_as_needed,
            &output,
            &passthrough,
            default_dirs,
        ),
        (Emulation::Aarch64, true) => lccc::linker_entry::link_shared_aarch64(
            &object_refs,
            &object_as_needed,
            &output,
            &passthrough,
            default_dirs,
        ),
        (Emulation::Aarch64, false) => lccc::linker_entry::link_builtin_aarch64(
            &object_refs,
            &object_as_needed,
            &output,
            &passthrough,
            default_dirs,
            is_static,
        ),
        (Emulation::Riscv64, true) => lccc::linker_entry::link_shared_riscv64(
            &object_refs,
            &object_as_needed,
            &output,
            &passthrough,
            default_dirs,
        ),
        (Emulation::Riscv64, false) => lccc::linker_entry::link_builtin_riscv64(
            &object_refs,
            &object_as_needed,
            &output,
            &passthrough,
            default_dirs,
        ),
    };
    res?;
    if oformat_binary {
        lccc::linker_entry::convert_elf_file_to_flat_binary(&output)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::handle_z_keyword;

    /// Space and joined `-z` spellings share one forwarding path; the shared
    /// linker-argument parser owns validation and page-size resolution.
    #[test]
    fn z_keyword_handler_forwards_page_size_spellings() {
        let mut passthrough = Vec::new();
        handle_z_keyword("relro", &mut passthrough);
        handle_z_keyword("max-page-size=0x1000", &mut passthrough);
        handle_z_keyword("max-page-size=0X10000", &mut passthrough);
        assert_eq!(
            passthrough,
            vec![
                "-Wl,-z,relro".to_string(),
                "-Wl,-z,max-page-size=0x1000".to_string(),
                "-Wl,-z,max-page-size=0X10000".to_string(),
            ]
        );
    }
}
