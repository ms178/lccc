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
/// chain): GNU ld accepts the two identically, and routing both through
/// this one function keeps them from drifting apart.  `max-page-size=` is
/// honoured locally (the script link reads it from here); every keyword,
/// including `max-page-size=`, is also forwarded so the shared pipeline's
/// `-z` parser sees exactly what the driver saw.
fn handle_z_keyword(
    kw: &str,
    max_page_size: &mut u64,
    max_page_size_explicit: &mut bool,
    passthrough: &mut Vec<String>,
) {
    if let Some(value) = kw.strip_prefix("max-page-size=") {
        *max_page_size_explicit = true;
        *max_page_size = if let Some(hex) = value.strip_prefix("0x") {
            u64::from_str_radix(hex, 16).unwrap_or(*max_page_size)
        } else {
            value.parse().unwrap_or(*max_page_size)
        };
    }
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
             the output image may differ from GNU ld",
            key
        );
    }
}

fn is_benign_ignorable(a: &str) -> bool {
    // gcc's driver state stack around --as-needed groups.
    if a == "--push-state" || a == "--pop-state" {
        return true;
    }
    // Diagnostic / bookkeeping switches with no layout effect.
    matches!(
        a,
        "--eh-frame-hdr"          // we always emit .eh_frame_hdr
        | "--no-add-needed"
        | "--no-copy-dt-needed-entries"
        | "--warn-common"
        | "--no-warn-mismatch"
        | "--no-warn-search-mismatch"
        | "--no-warn-execstack"
        | "--warn-execstack"
        | "--fatal-warnings"
        | "--no-fatal-warnings"
        | "--disable-linker-version"
        | "--no-relax"
        // Driver-level flags that reach a standalone linker when a build system
        // hands the same list to both. A linker never adds default libraries on
        // its own -- that is the driver's job -- so -nostdlib has no effect here,
        // and warning about it on every kernel-style link is noise that hides the
        // warnings that matter.
        | "-nostdlib" | "-nostartfiles" | "-nodefaultlibs"
        | "-O0" | "-O1" | "-O2" | "-O3" // ld's own -O is a size/speed hint
    ) || a.starts_with("-plugin-opt=")
}

fn run(args: &[String]) -> Result<(), String> {
    let mut output = "a.out".to_string();
    let mut script_path: Option<String> = None;
    let mut inputs: Vec<(String, bool)> = Vec::new(); // (path, whole_archive)
    let mut whole_archive = false;
    let mut emit_symtab = true;
    let mut relocatable = false;
    let mut emit_relocs = false;
    let mut gc_sections = false;
    let mut is_pie = false;
    let mut is_static = false;
    let mut build_id = false;
    let mut entry_override: Option<String> = None;
    let mut shared = false;
    let mut soname: Option<String> = None;
    let mut bsymbolic = false;
    let mut max_page_size = 0x200000u64;
    let mut max_page_size_explicit = false;
    let mut elf_i386 = false;
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

    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "-o" => {
                i += 1;
                output = args.get(i).cloned().ok_or("-o needs an argument")?;
            }
            "-m" => {
                // Emulation selects both the input parser and output ELF
                // class. ELF32 is currently supported for full script links,
                // which is the path used by Linux's real-mode setup image.
                i += 1;
                match args.get(i).map(String::as_str) {
                    Some("elf_x86_64") | Some("elf64_x86_64") => elf_i386 = false,
                    Some("elf_i386") | Some("i386linux") => elf_i386 = true,
                    None => return Err("-m needs an argument".into()),
                    Some(other) => {
                        return Err(format!("unsupported emulation '-m {other}'"));
                    }
                }
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
            "--emit-relocs" | "-q" => emit_relocs = true,
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
            "--strip-debug" | "-S" => {}
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
                println!("  Standard userspace, -r relocatable, and -T script links supported.");
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
                    handle_z_keyword(
                        kw,
                        &mut max_page_size,
                        &mut max_page_size_explicit,
                        &mut passthrough,
                    );
                }
            }
            "-static" | "-Bstatic" | "-dn" | "-non_shared" => {
                is_static = true;
                passthrough.push("-static".to_string());
            }
            "-Bdynamic" | "-dy" | "-call_shared" => {}
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
            "-Bsymbolic" | "-Bsymbolic-functions" => {
                bsymbolic = true;
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
                    // Two consumers: script links get a synthetic note object
                    // here, and the userspace pipeline gets the flag through
                    // the shared parser (it emits the note itself).  Previously
                    // only the script path was wired, so every `gcc
                    // -fuse-ld=lccc` build -- Debian's gcc passes --build-id on
                    // every link -- produced a binary with no build-id, which
                    // breaks debuginfod, distro debuginfo extraction and
                    // coredump matching.
                    build_id = !a.ends_with("=none") && !a.ends_with("=0");
                    passthrough.push(if let Some((_, v)) = a.split_once('=') {
                        format!("-Wl,--build-id={}", v)
                    } else {
                        "-Wl,--build-id".to_string()
                    });
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
                } else if a.starts_with("--hash-style") {
                    // Implemented: `gnu`, `sysv` and `both` all produce the
                    // matching `.gnu.hash` / `.hash` sections and DT tags.
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
                        handle_z_keyword(
                            kw,
                            &mut max_page_size,
                            &mut max_page_size_explicit,
                            &mut passthrough,
                        );
                    }
                } else if a.starts_with("--orphan-handling")
                    || a == "--no-warn-rwx-segments"
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
                    warn_unimplemented(a);
                } else if is_benign_ignorable(a) {
                    // Options every GNU-compatible linker accepts and that do
                    // not change the image we produce. bfd and mold accept
                    // these silently; warning about them buried real
                    // diagnostics under a dozen lines of noise on every single
                    // `gcc -fuse-ld=lccc` invocation, because gcc's driver
                    // always passes --push-state/--pop-state and the LTO
                    // plugin triplet.
                } else if a == "-plugin" || a.starts_with("-plugin-opt") {
                    // The LTO plugin is deliberately NOT silently ignored.
                    //
                    // gcc passes `-plugin liblto_plugin.so` unconditionally,
                    // but it only *matters* when an input is LTO bytecode
                    // rather than a real object. Ignoring it is correct for
                    // ordinary objects and silently wrong for `-flto` builds
                    // (the link would fail later with confusing "undefined
                    // symbol" errors, or quietly drop code). So: accept it,
                    // remember it, and let the object loader complain
                    // precisely if it ever meets an IR member.
                    if a == "-plugin" {
                        i += 1;
                    } // skip the plugin path
                    saw_lto_plugin = true;
                } else if a.starts_with('-') {
                    // Unknown flag: warn (parity with ld's permissiveness would
                    // be an error, but warn keeps us usable during bring-up).
                    eprintln!("lccc-ld: warning: ignoring unknown option '{}'", a);
                } else if whole_archive && a.ends_with(".a") {
                    // Positional archives under --whole-archive must go through
                    // the shared parser: it carries the positional state with
                    // the input and force-loads every member.  The plain
                    // `object_files` path always loads archives selectively.
                    passthrough.push("--whole-archive".to_string());
                    passthrough.push(a.to_string());
                    passthrough.push("--no-whole-archive".to_string());
                } else {
                    inputs.push((a.to_string(), whole_archive));
                }
            }
        }
        i += 1;
    }

    if inputs.is_empty() {
        return Err("no input files".into());
    }
    // i386's ABI maximum page size is 4 KiB. Preserve an explicit
    // `-z max-page-size=` override, but do not inherit x86-64's 2 MiB default:
    // doing so puts setup.elf's first 32 KiB section behind a 2 MiB file hole.
    if elf_i386 && !max_page_size_explicit {
        max_page_size = 0x1000;
    }

    // ------------------------------------------------------------------
    // Mode 1: relocatable link (ld -r).
    // ------------------------------------------------------------------
    if relocatable {
        if elf_i386 {
            return Err("ELF32/i386 relocatable (-r) output is not implemented; ELF32 is currently supported with -T/--script".into());
        }
        if script_path.is_some() {
            eprintln!("lccc-ld: warning: -r with a linker script: script ignored");
        }
        let mut objects = Vec::new();
        lccc::linker_entry::load_inputs_x86(&inputs, &mut objects, &undefined_symbols)?;
        return lccc::linker_entry::link_relocatable_x86(&objects, &output);
    }

    // GCC/Clang hand every link the LTO plugin. That is harmless for ordinary
    // objects, but if an input is actually LTO bytecode we cannot link it: the
    // plugin is what turns IR back into machine code, and lccc does not load
    // plugins. Detect it here and say so precisely, instead of letting the ELF
    // parser reject the file with a generic "not an ELF object" or — worse —
    // letting a `.o` that happens to parse produce a binary with missing code.
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
        let mut objects = if elf_i386 {
            lccc::linker_entry::load_inputs_i386_script(&inputs, &undefined_symbols)?
        } else {
            let mut objects = Vec::new();
            lccc::linker_entry::load_inputs_x86(&inputs, &mut objects, &undefined_symbols)?;
            objects
        };
        if build_id {
            lccc::linker_entry::append_build_id_object(&mut objects);
        }
        // GNU property note merge (CET/ISA) for script links: done here,
        // where the synthetic build-id object's index is known, so it is
        // excluded from the "real inputs" (a synthetic object without the
        // note would veto every AND-class type).  The merged note then
        // flows through the script layout like any other input section.
        let (cet_flags, z_ignored) = passthrough_property_flags(&passthrough)?;
        for kw in z_ignored {
            eprintln!("lccc-ld: warning: -z {kw} ignored");
        }
        let mut synthetic: lccc::linker_entry::FxHashSet<usize> = Default::default();
        if build_id {
            synthetic.insert(objects.len() - 1);
        }
        // `elf_i386` selects the note's entry stride (12-byte entries on
        // 32-bit targets, 16 on 64-bit ones — see `cet::parse_property_note`).
        if let Some(carrier) = lccc::linker_entry::merge_property_into_objects(
            &mut objects,
            &synthetic,
            &cet_flags,
            elf_i386,
        )? {
            synthetic.insert(objects.len());
            objects.push(carrier);
        }
        let mut script_src = std::fs::read_to_string(&script_path)
            .map_err(|e| format!("cannot read script '{}': {}", script_path, e))?;
        if let Some(e) = entry_override {
            // command-line -e overrides ENTRY() in the script
            script_src = format!("ENTRY({})\n{}", e, script_src);
        }
        if elf_i386 {
            return lccc::linker_entry::link_with_script_i386(
                &mut objects,
                &script_src,
                &output,
                emit_symtab,
                is_pie || shared,
                emit_relocs,
                gc_sections,
                soname.as_deref(),
                bsymbolic,
                max_page_size,
                &defsyms,
            );
        }
        return lccc::linker_entry::link_with_script_x86(
            &mut objects,
            &script_src,
            &output,
            emit_symtab,
            is_pie || shared,
            emit_relocs,
            gc_sections,
            soname.as_deref(),
            bsymbolic,
            max_page_size,
            &defsyms,
        );
    }

    if elf_i386 {
        return Err("ELF32/i386 output without a linker script is not implemented in lccc-ld; use the i686 compiler driver or pass -T".into());
    }

    // ------------------------------------------------------------------
    // Mode 3: standard userspace link — same pipeline as the compiler
    // driver (`link_builtin`/`link_shared`). CRT objects arrive as
    // positional inputs from the caller (gcc-style invocation), so no CRT
    // injection happens here; whole-archive members are force-loaded.
    // ------------------------------------------------------------------
    // Whole-archive archives were routed to `passthrough` during argument
    // parsing, so everything left here is an ordinary object or archive.
    let mut object_files: Vec<String> = Vec::new();
    for (path, _wa) in &inputs {
        object_files.push(path.clone());
    }
    let object_refs: Vec<&str> = object_files.iter().map(|s| s.as_str()).collect();

    if shared {
        return lccc::linker_entry::link_shared_x86(&object_refs, &output, &passthrough);
    }
    // A plain -pie is honoured by the built-in emitter: ET_DYN based at 0,
    // DF_1_PIE, and an R_X86_64_RELATIVE for every internal absolute address,
    // so the kernel may map it anywhere and ld.so slides it correctly.
    //
    // -static-pie is still refused.  A static PIE needs the same RELATIVE
    // table *plus* the rcrt1.o self-relocation protocol, and the static emitter
    // produces no .rela.dyn at all; writing the image anyway yields something
    // that faults in the CRT before main, which is worse than saying no.
    if is_pie && is_static {
        return Err("-static-pie is not implemented: lccc-ld cannot yet emit a \
             position-independent static executable (refusing rather than \
             producing an image that faults in the CRT self-relocation)"
            .to_string());
    }
    lccc::linker_entry::link_builtin_x86(&object_refs, &output, &passthrough)
}

#[cfg(test)]
mod tests {
    use super::handle_z_keyword;

    /// Both `-z` spellings funnel through `handle_z_keyword`, so the
    /// joined form (`-zrelro`) forwards exactly like the space form
    /// (`-z relro`), and `max-page-size=` (decimal or hex) is honoured
    /// locally either way.
    #[test]
    fn z_keyword_handler_forwards_and_honours_page_size() {
        let mut max_page_size = 0x200000u64;
        let mut explicit = false;
        let mut passthrough = Vec::new();
        handle_z_keyword("relro", &mut max_page_size, &mut explicit, &mut passthrough);
        handle_z_keyword(
            "max-page-size=0x1000",
            &mut max_page_size,
            &mut explicit,
            &mut passthrough,
        );
        assert_eq!(
            passthrough,
            vec![
                "-Wl,-z,relro".to_string(),
                "-Wl,-z,max-page-size=0x1000".to_string()
            ]
        );
        assert!(explicit);
        assert_eq!(max_page_size, 0x1000);

        let mut max_page_size = 0x200000u64;
        let mut explicit = false;
        let mut passthrough = Vec::new();
        handle_z_keyword(
            "max-page-size=4096",
            &mut max_page_size,
            &mut explicit,
            &mut passthrough,
        );
        assert!(explicit);
        assert_eq!(max_page_size, 4096);
    }
}
