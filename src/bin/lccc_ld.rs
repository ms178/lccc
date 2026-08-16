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
    match run(&args) {
        Ok(()) => {}
        Err(e) => {
            eprintln!("lccc-ld: error: {}", e);
            std::process::exit(1);
        }
    }
}

fn run(args: &[String]) -> Result<(), String> {
    let mut output = "a.out".to_string();
    let mut script_path: Option<String> = None;
    let mut inputs: Vec<(String, bool)> = Vec::new(); // (path, whole_archive)
    let mut whole_archive = false;
    let mut emit_symtab = true;
    let mut relocatable = false;
    let mut is_pie = false;
    let mut build_id = false;
    let mut entry_override: Option<String> = None;
    let mut shared = false;
    // Arguments forwarded verbatim into the builtin userspace pipeline
    // (parse_linker_args understands the GNU spellings directly).
    let mut passthrough: Vec<String> = Vec::new();

    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "-o" => { i += 1; output = args.get(i).cloned().ok_or("-o needs an argument")?; }
            "-m" => { i += 1; /* emulation: elf_x86_64 assumed */ }
            "-T" | "--script" => {
                i += 1;
                script_path = Some(args.get(i).cloned().ok_or("-T needs an argument")?);
            }
            "-r" | "--relocatable" | "-i" => relocatable = true,
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
            "-v" | "--version" => {
                println!("LCCC ld (GNU-compatible) 0.1");
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
            "-Map" => { i += 1; }
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
                    passthrough.push(format!("-Wl,-z,{}", kw));
                }
            }
            "-static" | "-Bstatic" | "-dn" | "-non_shared" => {
                passthrough.push("-static".to_string());
            }
            "-Bdynamic" | "-dy" | "-call_shared" => {}
            "--gc-sections" => passthrough.push("-Wl,--gc-sections".to_string()),
            "--no-gc-sections" => {}
            "--export-dynamic" | "-E" => passthrough.push("-rdynamic".to_string()),
            "--as-needed" | "--no-as-needed" | "--eh-frame-hdr"
            | "--fix-cortex-a53-843419" | "--no-copy-dt-needed-entries"
            | "--allow-shlib-undefined" | "-X" | "-x" => {}
            _ => {
                if let Some(v) = a.strip_prefix("--script=") {
                    script_path = Some(v.to_string());
                } else if let Some(v) = a.strip_prefix("-T") {
                    if !v.is_empty() { script_path = Some(v.to_string()); }
                } else if let Some(v) = a.strip_prefix("--entry=") {
                    entry_override = Some(v.to_string());
                    passthrough.push(a.to_string());
                } else if let Some(v) = a.strip_prefix("-Map=") {
                    let _ = v;
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
                        passthrough.push(format!("-Wl,-soname,{}", val));
                    }
                } else if a.starts_with("--build-id") {
                    build_id = !a.ends_with("=none");
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
                    || a == "--no-warn-rwx-segments" || a.starts_with("-z")
                    || a.starts_with("--hash-style")
                    || a.starts_with("--sort-section")
                    || a.starts_with("--print-")
                    || a.starts_with("--emit-relocs") {
                    // accepted, not needed for correctness of the static image
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

    // ------------------------------------------------------------------
    // Mode 1: relocatable link (ld -r).
    // ------------------------------------------------------------------
    if relocatable {
        if script_path.is_some() {
            eprintln!("lccc-ld: warning: -r with a linker script: script ignored");
        }
        let mut objects = Vec::new();
        lccc::linker_entry::load_inputs_x86(&inputs, &mut objects)?;
        return lccc::linker_entry::link_relocatable_x86(&objects, &output);
    }

    // ------------------------------------------------------------------
    // Mode 2: script-driven link (kernel-style -T).
    // ------------------------------------------------------------------
    if let Some(script_path) = script_path {
        let mut objects = Vec::new();
        lccc::linker_entry::load_inputs_x86(&inputs, &mut objects)?;
        if build_id {
            lccc::linker_entry::append_build_id_object(&mut objects);
        }
        let mut script_src = std::fs::read_to_string(&script_path)
            .map_err(|e| format!("cannot read script '{}': {}", script_path, e))?;
        if let Some(e) = entry_override {
            // command-line -e overrides ENTRY() in the script
            script_src = format!("ENTRY({})\n{}", e, script_src);
        }
        return lccc::linker_entry::link_with_script_x86(
            &objects, &script_src, &output, emit_symtab, is_pie);
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
