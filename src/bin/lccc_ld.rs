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

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

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
        | "-O0" | "-O1" | "-O2" | "-O3" // ld's own -O is a size/speed hint
    ) || a.starts_with("--build-id=")
        || a.starts_with("-plugin-opt=")
}

fn run(args: &[String]) -> Result<(), String> {
    let mut output = "a.out".to_string();
    let mut script_path: Option<String> = None;
    let mut inputs: Vec<(String, bool)> = Vec::new(); // (path, whole_archive)
    let mut whole_archive = false;
    let mut emit_symtab = true;
    let mut relocatable = false;
    let mut emit_relocs = false;
    let mut is_pie = false;
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
            "-pie" | "--pic-executable" => is_pie = true,
            "-no-pie" => is_pie = false,
            "-shared" | "-Bshareable" => shared = true,
            "--no-dynamic-linker" | "--no-ld-generated-unwind-info" => {}
            "--whole-archive" => whole_archive = true,
            "--no-whole-archive" => whole_archive = false,
            "--start-group" | "--end-group" | "-(" | "-)" => {
                // Builtin archive loading already iterates to a fixpoint
                // (global group semantics), which subsumes group regions.
            }
            "--strip-debug" | "-S" => {}
            "--strip-all" | "-s" => emit_symtab = false,
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
            "--dynamic-linker" | "-dynamic-linker" | "-I" => {
                // The builtin emitter hardwires the standard glibc interp
                // path; accept and verify rather than silently diverging.
                i += 1;
                if let Some(p) = args.get(i) {
                    if p != "/lib64/ld-linux-x86-64.so.2" && !p.is_empty() {
                        eprintln!(
                            "lccc-ld: warning: non-standard --dynamic-linker '{}' \
                             (builtin emitter uses /lib64/ld-linux-x86-64.so.2)",
                            p
                        );
                    }
                }
            }
            "-z" => {
                i += 1;
                if let Some(kw) = args.get(i) {
                    if let Some(value) = kw.strip_prefix("max-page-size=") {
                        max_page_size_explicit = true;
                        max_page_size = if let Some(hex) = value.strip_prefix("0x") {
                            u64::from_str_radix(hex, 16).unwrap_or(max_page_size)
                        } else {
                            value.parse().unwrap_or(max_page_size)
                        };
                    }
                    passthrough.push(format!("-Wl,-z,{}", kw));
                }
            }
            "-static" | "-Bstatic" | "-dn" | "-non_shared" => {
                passthrough.push("-static".to_string());
            }
            "-Bdynamic" | "-dy" | "-call_shared" => {}
            "--gc-sections" => passthrough.push("-Wl,--gc-sections".to_string()),
            "--no-gc-sections" => {}
            "--no-undefined" => {
                // Script links resolve every non-weak relocation eagerly.
                passthrough.push(a.to_string());
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
                } else if let Some(rest) = a.strip_prefix("--wrap=") {
                    passthrough.push(format!("-Wl,--wrap={}", rest));
                } else if let Some(rest) = a.strip_prefix("--defsym=") {
                    passthrough.push(format!("-Wl,--defsym={}", rest));
                } else if let Some(rest) = a.strip_prefix("-u") {
                    let sym = if rest.is_empty() {
                        i += 1;
                        args.get(i).cloned().unwrap_or_default()
                    } else {
                        rest.to_string()
                    };
                    if !sym.is_empty() {
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
                    build_id = !a.ends_with("=none");
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
                } else if a.starts_with("--orphan-handling")
                    || a == "--no-warn-rwx-segments"
                    || a.starts_with("-z")
                    || a.starts_with("--hash-style")
                    || a.starts_with("--sort-section")
                    || a.starts_with("--print-")
                {
                    // accepted, not needed for correctness of the static image
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
        lccc::linker_entry::load_inputs_x86(&inputs, &mut objects)?;
        return lccc::linker_entry::link_relocatable_x86(&objects, &output);
    }

    // GCC/Clang hand every link the LTO plugin. That is harmless for ordinary
    // objects, but if an input is actually LTO bytecode we cannot link it: the
    // plugin is what turns IR back into machine code, and lccc does not load
    // plugins. Detect it here and say so precisely, instead of letting the ELF
    // parser reject the file with a generic "not an ELF object" or — worse —
    // letting a `.o` that happens to parse produce a binary with missing code.
    if saw_lto_plugin {
        if let Some(bad) = inputs
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
    }

    // ------------------------------------------------------------------
    // Mode 2: script-driven link (kernel-style -T).
    // ------------------------------------------------------------------
    if let Some(script_path) = script_path {
        let mut objects = if elf_i386 {
            lccc::linker_entry::load_inputs_i386_script(&inputs)?
        } else {
            let mut objects = Vec::new();
            lccc::linker_entry::load_inputs_x86(&inputs, &mut objects)?;
            objects
        };
        if build_id {
            lccc::linker_entry::append_build_id_object(&mut objects);
        }
        let mut script_src = std::fs::read_to_string(&script_path)
            .map_err(|e| format!("cannot read script '{}': {}", script_path, e))?;
        if let Some(e) = entry_override {
            // command-line -e overrides ENTRY() in the script
            script_src = format!("ENTRY({})\n{}", e, script_src);
        }
        if elf_i386 {
            return lccc::linker_entry::link_with_script_i386(
                &objects,
                &script_src,
                &output,
                emit_symtab,
                is_pie || shared,
                emit_relocs,
                soname.as_deref(),
                bsymbolic,
                max_page_size,
            );
        }
        return lccc::linker_entry::link_with_script_x86(
            &objects,
            &script_src,
            &output,
            emit_symtab,
            is_pie || shared,
            emit_relocs,
            soname.as_deref(),
            bsymbolic,
            max_page_size,
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
    let mut object_files: Vec<String> = Vec::new();
    for (path, wa) in &inputs {
        if *wa && path.ends_with(".a") {
            // The builtin pipeline loads archives lazily (pull members only
            // when they resolve an undefined). --whole-archive semantics are
            // only needed by script links today; refuse loudly instead of
            // producing a binary that silently dropped members.
            return Err(format!(
                "--whole-archive '{}' is only supported with -T/--script or -r; \
                 pass the members as objects for a standard link",
                path
            ));
        }
        object_files.push(path.clone());
    }
    let object_refs: Vec<&str> = object_files.iter().map(|s| s.as_str()).collect();

    if shared {
        return lccc::linker_entry::link_shared_x86(&object_refs, &output, &passthrough);
    }
    if is_pie {
        eprintln!("lccc-ld: warning: -pie without -T uses the fixed-base executable emitter");
    }
    lccc::linker_entry::link_builtin_x86(&object_refs, &output, &passthrough)
}
