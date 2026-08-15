//! `lccc-ld`: GNU-ld-compatible command-line driver for the LCCC linker.
//!
//! Supports the invocation styles used by large build systems that call the
//! linker directly (most importantly the Linux kernel's
//! `scripts/link-vmlinux.sh`):
//!
//! ```text
//! lccc-ld -m elf_x86_64 -z noexecstack --script=vmlinux.lds -o vmlinux \
//!         --whole-archive vmlinux.a --no-whole-archive \
//!         --start-group --end-group extra.o ...
//! ```
//!
//! When a full `SECTIONS` script is given via `-T`/`--script`, the link is
//! performed by the script-driven layout engine (`emit_script`). Without a
//! script it falls back to the standard built-in executable emitter.

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
    let mut entry_override: Option<String> = None;

    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "-o" => { i += 1; output = args.get(i).cloned().ok_or("-o needs an argument")?; }
            "-m" => { i += 1; /* emulation: elf_x86_64 assumed */ }
            "-z" => { i += 1; /* -z keywords: accepted */ }
            "-T" | "--script" => {
                i += 1;
                script_path = Some(args.get(i).cloned().ok_or("-T needs an argument")?);
            }
            "-r" | "--relocatable" | "-i" => relocatable = true,
            "--whole-archive" => whole_archive = true,
            "--no-whole-archive" => whole_archive = false,
            "--start-group" | "--end-group" | "-(" | "-)" => {}
            "--strip-debug" | "-S" => {}
            "--strip-all" | "-s" => emit_symtab = false,
            "-v" | "--version" => {
                println!("LCCC ld (GNU-compatible) 0.1");
                return Ok(());
            }
            "--help" => {
                println!("Usage: lccc-ld [options] file...");
                return Ok(());
            }
            "-e" | "--entry" => {
                i += 1;
                entry_override = args.get(i).cloned();
            }
            "-Map" => { i += 1; }
            _ => {
                if let Some(v) = a.strip_prefix("--script=") {
                    script_path = Some(v.to_string());
                } else if let Some(v) = a.strip_prefix("-T") {
                    if !v.is_empty() { script_path = Some(v.to_string()); }
                } else if let Some(v) = a.strip_prefix("--entry=") {
                    entry_override = Some(v.to_string());
                } else if let Some(v) = a.strip_prefix("-Map=") {
                    let _ = v;
                } else if a.starts_with("--orphan-handling") || a.starts_with("--build-id")
                    || a == "--no-warn-rwx-segments" || a.starts_with("-z")
                    || a.starts_with("--hash-style") || a == "--as-needed"
                    || a == "--no-as-needed" || a == "--eh-frame-hdr"
                    || a.starts_with("--sort-section") || a == "--gc-sections"
                    || a.starts_with("--print-") || a == "-X" || a == "-x"
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

    // Load all inputs.
    let mut objects = Vec::new();
    lccc::linker_entry::load_inputs_x86(&inputs, &mut objects)?;

    if relocatable {
        if script_path.is_some() {
            eprintln!("lccc-ld: warning: -r with a linker script: script ignored");
        }
        return lccc::linker_entry::link_relocatable_x86(&objects, &output);
    }

    let Some(script_path) = script_path else {
        return Err("lccc-ld currently requires a linker script (-T/--script) or -r; \
                    use the lccc driver for standard userspace links".into());
    };
    let mut script_src = std::fs::read_to_string(&script_path)
        .map_err(|e| format!("cannot read script '{}': {}", script_path, e))?;
    if let Some(e) = entry_override {
        // command-line -e overrides ENTRY() in the script
        script_src = format!("ENTRY({})\n{}", e, script_src);
    }

    lccc::linker_entry::link_with_script_x86(&objects, &script_src, &output, emit_symtab)
}
