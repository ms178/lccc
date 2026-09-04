//! CLI argument parsing for GCC-compatible command-line flags.
//!
//! Handles the full range of GCC flags that build systems like the Linux kernel,
//! Meson, and autoconf expect: optimization levels, debug info, preprocessor
//! directives, linker flags, target-specific machine flags, and query flags
//! like `--dumpmachine` and `--version`.
//!
//! Design: The parser is a simple `while` loop with a flat `match` on each
//! argument. No external parser library is used. Unknown flags are silently
//! ignored (matching GCC's behavior for unrecognized `-f` and `-m` flags),
//! which is critical for build system compatibility.

use super::pipeline::{CliDefine, CompileMode, Driver};
use crate::backend::Target;
use crate::common::error::ColorMode;
use crate::common::fp_contract::FpContract;

/// Compare dotted version strings numerically ("16.1.1" > "9.5.0").
///
/// A lexical sort orders "9.5.0" AFTER "16.1.1"/"12", so
/// `-print-libgcc-file-name` would probe the OLDEST GCC directory first
/// instead of the newest. Version directories are dotted decimal, so a
/// component-wise numeric comparison is the correct order.
fn cmp_version(a: &str, b: &str) -> std::cmp::Ordering {
    let mut ai = a.split('.');
    let mut bi = b.split('.');
    loop {
        match (ai.next(), bi.next()) {
            (None, None) => return std::cmp::Ordering::Equal,
            (None, Some(_)) => return std::cmp::Ordering::Less,
            (Some(_), None) => return std::cmp::Ordering::Greater,
            (Some(x), Some(y)) => match x
                .parse::<u64>()
                .unwrap_or(0)
                .cmp(&y.parse::<u64>().unwrap_or(0))
            {
                std::cmp::Ordering::Equal => continue,
                ord => return ord,
            },
        }
    }
}

impl Driver {
    fn enable_x86_avx_profile(&mut self) {
        // Explicit `-mno-sse` is sticky against `-march=` CPU profiles (GCC).
        // Do not set AVX/SSE feature bits either: `avx2_enabled` would otherwise
        // still select `vmovdqu` for 64-byte copies while CR4.OSFXSR=0.
        if self.sse_explicitly_disabled {
            return;
        }
        self.no_sse = false;
        self.enable_avx = true;
        self.enable_sse4_2 = true;
        self.enable_sse4_1 = true;
        self.enable_ssse3 = true;
        self.enable_sse3 = true;
    }
    fn enable_x86_avx2_profile(&mut self) {
        self.enable_x86_avx_profile();
        if self.sse_explicitly_disabled {
            return;
        }
        self.enable_avx2 = true;
    }
    fn enable_x86_v3_profile(&mut self) {
        self.enable_x86_avx2_profile();
        // BMI/LZCNT/MOVBE are integer ISA and remain legal under `-mno-sse`.
        self.enable_bmi = true;
        self.enable_bmi2 = true;
        self.enable_lzcnt = true;
        self.enable_movbe = true;
        // x86-64-v3 is a superset of v2, which added POPCNT.
        self.enable_popcnt = true;
        if self.sse_explicitly_disabled {
            return;
        }
        self.enable_f16c = true;
        self.enable_fma = true;
    }
    fn enable_x86_haswell_profile(&mut self) {
        self.enable_x86_v3_profile();
        if self.sse_explicitly_disabled {
            return;
        }
        self.enable_aes = true;
        self.enable_pclmul = true;
        self.enable_rdrnd = true;
    }
    fn enable_x86_avx512_profile(&mut self) {
        self.enable_x86_v3_profile();
        if self.sse_explicitly_disabled {
            return;
        }
        self.enable_avx512f = true;
        self.enable_avx512cd = true;
        self.enable_avx512dq = true;
        self.enable_avx512bw = true;
        self.enable_avx512vl = true;
    }
    fn enable_x86_avx512_cpu_profile(&mut self) {
        self.enable_x86_avx512_profile();
        self.enable_aes = true;
        self.enable_pclmul = true;
        self.enable_rdrnd = true;
    }
    fn enable_x86_cascadelake_profile(&mut self) {
        self.enable_x86_avx512_cpu_profile();
        self.enable_avx512vnni = true;
    }
    fn enable_x86_cooperlake_profile(&mut self) {
        self.enable_x86_cascadelake_profile();
        self.enable_avx512bf16 = true;
    }
    fn enable_x86_icelake_profile(&mut self) {
        self.enable_x86_cascadelake_profile();
        self.enable_avx512ifma = true;
        self.enable_avx512vbmi = true;
        self.enable_avx512vbmi2 = true;
        self.enable_avx512bitalg = true;
        self.enable_avx512vpopcntdq = true;
        self.enable_gfni = true;
        self.enable_vaes = true;
        self.enable_vpclmulqdq = true;
    }
    fn enable_x86_sapphirerapids_profile(&mut self) {
        self.enable_x86_icelake_profile();
        self.enable_avx512bf16 = true;
        self.enable_avx512fp16 = true;
    }
    fn enable_x86_knl_profile(&mut self) {
        self.enable_x86_v3_profile();
        if self.sse_explicitly_disabled {
            return;
        }
        self.enable_pclmul = true;
        self.enable_rdrnd = true;
        self.enable_avx512f = true;
        self.enable_avx512cd = true;
        self.enable_avx512er = true;
        self.enable_avx512pf = true;
    }
    fn enable_x86_znver3_profile(&mut self) {
        self.enable_x86_haswell_profile();
        if self.sse_explicitly_disabled {
            return;
        }
        self.enable_vaes = true;
        self.enable_vpclmulqdq = true;
    }
    fn enable_x86_znver4_profile(&mut self) {
        self.enable_x86_icelake_profile();
        self.enable_avx512bf16 = true;
    }
    fn enable_x86_znver5_profile(&mut self) {
        self.enable_x86_znver4_profile();
        self.enable_avxvnni = true;
        self.enable_avx512vp2intersect = true;
    }
    fn enable_x86_nehalem_profile(&mut self) {
        // POPCNT shipped with Nehalem/Barcelona and is part of the
        // x86-64-v2 baseline contract; like BMI/LZCNT it is integer ISA
        // and stays legal under `-mno-sse`.
        self.enable_popcnt = true;
        if self.sse_explicitly_disabled {
            return;
        }
        self.no_sse = false;
        self.enable_sse3 = true;
        self.enable_ssse3 = true;
        self.enable_sse4_1 = true;
        self.enable_sse4_2 = true;
    }
    fn enable_x86_westmere_profile(&mut self) {
        self.enable_x86_nehalem_profile();
        self.enable_aes = true;
        self.enable_pclmul = true;
    }
    fn enable_x86_sandybridge_profile(&mut self) {
        self.enable_x86_westmere_profile();
        self.enable_x86_avx_profile();
    }
    fn enable_x86_ivybridge_profile(&mut self) {
        self.enable_x86_sandybridge_profile();
        self.enable_f16c = true;
        self.enable_rdrnd = true;
    }
    fn enable_x86_silvermont_profile(&mut self) {
        self.enable_x86_nehalem_profile();
        self.enable_pclmul = true;
        self.enable_movbe = true;
        self.enable_rdrnd = true;
    }
    fn enable_x86_goldmont_profile(&mut self) {
        self.enable_x86_silvermont_profile();
        self.enable_aes = true;
    }
    fn enable_x86_alderlake_profile(&mut self) {
        self.enable_x86_haswell_profile();
        if self.sse_explicitly_disabled {
            return;
        }
        self.enable_avxvnni = true;
        self.enable_gfni = true;
        self.enable_vaes = true;
        self.enable_vpclmulqdq = true;
    }
    fn enable_x86_arrowlake_profile(&mut self) {
        self.enable_x86_alderlake_profile();
        self.enable_avxifma = true;
        self.enable_avxneconvert = true;
        self.enable_avxvnniint8 = true;
        self.enable_cmpccxadd = true;
    }

    /// Parse GCC-compatible command-line arguments and populate driver fields.
    /// Returns `Ok(true)` if early exit was handled (query flags like -dumpmachine),
    /// `Ok(false)` if normal compilation should proceed, or `Err` for invalid args.
    pub fn parse_cli_args(&mut self, args: &[String]) -> Result<bool, String> {
        // Detect target from binary name (argv[0])
        let binary_name = std::path::Path::new(&args[0])
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("ccc");

        self.target = if binary_name.contains("arm") || binary_name.contains("aarch64") {
            Target::Aarch64
        } else if binary_name.contains("riscv") {
            Target::Riscv64
        } else if binary_name.contains("i686") || binary_name.contains("i386") {
            Target::I686
        } else {
            Target::X86_64
        };
        // Plain `char` signedness is target-defined (C11 6.7.2p5): unsigned
        // on AArch64/RISC-V SysV, signed on x86. Recorded process-globally
        // BEFORE any parsing/sema/lowering so every type resolution, cast,
        // and character constant sees the target's answer. -funsigned-char /
        // -fsigned-char override the default in the argument loop.
        crate::common::types::set_char_unsigned(matches!(
            self.target,
            Target::Aarch64 | Target::Riscv64
        ));

        // Handle GCC query flags that exit immediately (before requiring input files).
        // These are used by configure scripts to detect the compiler and target.
        if Self::handle_query_flags(args, &self.target)? {
            return Ok(true);
        }

        // Expand @response_file arguments (GCC/MSVC convention).
        // Response files contain additional command-line arguments, one per line
        // or whitespace-separated. Build systems like Meson use them when the
        // command line would exceed OS limits.
        let expanded_args = Self::expand_response_files(&args[1..]);
        self.parse_main_args(&expanded_args)?;

        // Store raw args for GCC -m16 passthrough. We keep everything except
        // argv[0], -o <output>, -c/-S/-E (we set mode ourselves), and input files.
        // GCC understands all the same flags we accept, so forwarding them directly
        // preserves ordering semantics (e.g., -fcf-protection=none after =branch).
        if self.code16gcc {
            self.raw_args = args[1..]
                .iter()
                .filter(|a| !self.input_files.contains(a))
                .cloned()
                .collect();
        }

        // Special case: no input files but -Wl,--version is present.
        // Build systems like Meson run `compiler -Wl,--version` without source files
        // to detect the linker type. Invoke our linker driver (GCC) directly.
        if self.input_files.is_empty()
            && self
                .linker_ordered_items
                .iter()
                .any(|a| a.contains("--version"))
        {
            Self::run_linker_version_query(&self.target, &self.linker_ordered_items);
            return Ok(true);
        }

        Ok(false)
    }

    /// Handle early-exit query flags (--dumpmachine, --version, etc.).
    /// Returns Ok(true) if a query flag was handled and the process should exit.
    fn handle_query_flags(args: &[String], target: &Target) -> Result<bool, String> {
        // Capture --sysroot / -isysroot so -print-libgcc-file-name resolves
        // against the sysroot when one is given (glibc cross-style builds).
        let mut sysroot: Option<String> = None;
        {
            let mut it = args[1..].iter();
            while let Some(a) = it.next() {
                if let Some(v) = a.strip_prefix("--sysroot=") {
                    if !v.is_empty() {
                        sysroot = Some(v.to_string());
                    }
                } else if a == "--sysroot" || a == "-isysroot" {
                    if let Some(v) = it.next() {
                        sysroot = Some(v.clone());
                    }
                }
            }
        }
        for arg in &args[1..] {
            match arg.as_str() {
                "-dumpmachine" => {
                    println!("{}", target.triple());
                    return Ok(true);
                }
                "-dumpversion" => {
                    println!("14");
                    return Ok(true);
                }
                "-print-libgcc-file-name" | "--print-libgcc-file-name" => {
                    // GCC query used by configure scripts (e.g. glibc's "usable
                    // compiler runtime library" probe) to decide between a
                    // libgcc and a compiler-rt runtime. Probe the same GCC lib
                    // directories the built-in linker searches; fall back to a
                    // bare name (glibc only inspects the basename).
                    let mut search_dirs: Vec<String> = Vec::new();
                    if let Some(ref sroot) = sysroot {
                        for triple_dir in [
                            format!("{}/usr/lib/gcc/{}/", sroot, target.triple()),
                            format!("{}/usr/lib/gcc/x86_64-pc-linux-gnu/", sroot),
                        ] {
                            if let Ok(rd) = std::fs::read_dir(&triple_dir) {
                                let mut vers: Vec<String> = rd
                                    .filter_map(|e| e.ok())
                                    .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
                                    .map(|e| e.file_name().to_string_lossy().to_string())
                                    .filter(|n| {
                                        n.chars()
                                            .next()
                                            .map(|c| c.is_ascii_digit())
                                            .unwrap_or(false)
                                    })
                                    .collect();
                                // Numeric version order: newest GCC dir first.
                                vers.sort_by(|a, b| cmp_version(b, a));
                                for v in vers {
                                    search_dirs.push(format!("{}{}/", triple_dir, v));
                                }
                            }
                        }
                        search_dirs.push(format!("{}/usr/lib/", sroot));
                    }
                    let triples = [
                        "x86_64-linux-gnu",
                        "x86_64-pc-linux-gnu",
                        "x86_64-redhat-linux",
                        "x86_64-linux",
                    ];
                    let mut printed = false;
                    for dir in &search_dirs {
                        let a = format!("{}libgcc.a", dir);
                        let so = format!("{}libgcc_s.so", dir);
                        if std::path::Path::new(&a).exists() || std::path::Path::new(&so).exists() {
                            println!("{}", a);
                            printed = true;
                            break;
                        }
                    }
                    for triple in triples {
                        let dir = format!("/usr/lib/gcc/{}", triple);
                        if let Ok(entries) = std::fs::read_dir(&dir) {
                            let mut vers: Vec<String> = entries
                                .filter_map(|e| e.ok())
                                .filter_map(|e| e.file_name().into_string().ok())
                                .filter(|n| n.chars().all(|c| c.is_ascii_digit() || c == '.'))
                                .collect();
                            vers.sort_by(|a, b| cmp_version(b, a));
                            for v in vers.iter() {
                                let a = format!("{}/{}/libgcc.a", dir, v);
                                let s = format!("{}/{}/libgcc_s.so", dir, v);
                                if std::path::Path::new(&a).exists()
                                    || std::path::Path::new(&s).exists()
                                {
                                    println!("{}", a);
                                    printed = true;
                                    break;
                                }
                            }
                            if printed {
                                break;
                            }
                        }
                    }
                    if !printed {
                        println!("libgcc.a");
                    }
                    return Ok(true);
                }
                _ if arg.starts_with("-print-file-name=")
                    || arg.starts_with("--print-file-name=") =>
                {
                    // glibc link rules: `${CC} --print-file-name=crtbeginS.o`.
                    let name = arg.trim_start_matches('-').trim_start_matches('-');
                    let name = name.strip_prefix("print-file-name=").unwrap_or(name);
                    if name == "include" {
                        if let Some(bundled) =
                            crate::frontend::preprocessor::Preprocessor::bundled_include_dir()
                        {
                            println!("{}", bundled.display());
                            return Ok(true);
                        }
                    }
                    let mut search_dirs: Vec<String> = Vec::new();
                    if let Some(ref sroot) = sysroot {
                        for gcc_triple in ["x86_64-pc-linux-gnu", target.triple()] {
                            let base = format!("{}/usr/lib/gcc/{}/", sroot, gcc_triple);
                            search_dirs.push(base.clone());
                            if let Ok(rd) = std::fs::read_dir(&base) {
                                let mut vers: Vec<String> = rd
                                    .filter_map(|e| e.ok())
                                    .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
                                    .map(|e| e.file_name().to_string_lossy().to_string())
                                    .filter(|n| {
                                        n.chars()
                                            .next()
                                            .map(|c| c.is_ascii_digit())
                                            .unwrap_or(false)
                                    })
                                    .collect();
                                vers.sort();
                                vers.reverse();
                                for v in vers {
                                    search_dirs.push(format!("{}{}/", base, v));
                                }
                            }
                        }
                        search_dirs.push(format!("{}/usr/lib/", sroot));
                    }
                    let triple = target.triple();
                    search_dirs.push(format!("/usr/lib/gcc/{}/13/", triple));
                    search_dirs.push(format!("/usr/lib/{}/", triple));
                    search_dirs.push("/usr/lib/".to_string());
                    let mut found = false;
                    for dir in &search_dirs {
                        let path = format!("{}{}", dir, name);
                        if std::path::Path::new(&path).exists() {
                            println!("{}", path);
                            found = true;
                            break;
                        }
                    }
                    if !found {
                        println!("{}", name);
                    }
                    return Ok(true);
                }
                "--version" => {
                    // Meson detects GCC by checking for "Free Software Foundation"
                    // in the --version output. We claim GCC 14.2.0 compatibility
                    // (matching our __GNUC__/__GNUC_MINOR__/__GNUC_PATCHLEVEL__).
                    println!(
                        "lccc (a high performance Claude's C Compiler fork, GCC-compatible) 14.2.0"
                    );
                    println!("GCC is maintained by the Free Software Foundation, Inc.");
                    println!("This program is a fork of CCC (written by Claude Opus 4.6);");
                    println!("It is not intended for production use.");
                    // Show which GCC fallback features are enabled (if any)
                    let mut features = Vec::new();
                    if cfg!(feature = "gcc_linker") {
                        features.push("gcc_linker");
                    }
                    if cfg!(feature = "gcc_assembler") {
                        features.push("gcc_assembler");
                    }
                    if cfg!(feature = "gcc_m16") {
                        features.push("gcc_m16");
                    }
                    if features.is_empty() {
                        println!("Backend: standalone");
                    } else {
                        println!("Backend: {}", features.join(", "));
                    }
                    return Ok(true);
                }
                "-v" if args.len() == 2 => {
                    println!(
                        "lccc (a high performance Claude's C Compiler fork, GCC-compatible) 14.2.0"
                    );
                    // GCC-style line so configure scripts that grep `-v` output
                    // for "gcc" (e.g. zlib-ng's compiler detection) classify the
                    // driver as GCC-compatible and enable the -fPIC/-std paths.
                    println!("gcc version 14.2.0 (lccc)");
                    println!("Target: {}", target.triple());
                    return Ok(true);
                }
                "-print-multi-directory" | "--print-multi-directory" => {
                    // GCC query returning the compiler's multilib directory.
                    // glibc's configure reads `multidir` from this; an empty
                    // answer makes its csu rules mis-expand (`; ; ln -s .`).
                    println!(".");
                    return Ok(true);
                }
                _ if arg.starts_with("-print-prog-name=")
                    || arg.starts_with("--print-prog-name=") =>
                {
                    // GCC query returning the path of a helper program.
                    // glibc's configure derives LD from `$CC -print-prog-name=ld`
                    // and then requires `$LD --version` to advertise GNU ld;
                    // without this answer LD ends up empty and configure aborts
                    // with "These critical programs are missing: GNU ld".
                    //
                    // The linker programs resolve to the sibling lccc-ld binary
                    // (which advertises "GNU ld (LCCC built-in)"). Everything
                    // else follows GCC's not-found behaviour: print the plain
                    // name unchanged.
                    let name = arg.trim_start_matches('-');
                    let name = name.strip_prefix("print-prog-name=").unwrap_or(name);
                    match name {
                        "ld" | "ld.bfd" | "collect2" => {
                            let sibling = std::env::current_exe()
                                .ok()
                                .and_then(|p| p.parent().map(|d| d.join("lccc-ld")))
                                .filter(|p| p.exists());
                            match sibling {
                                Some(p) => println!("{}", p.display()),
                                None => println!("lccc-ld"),
                            }
                        }
                        other => println!("{}", other),
                    }
                    return Ok(true);
                }
                "-print-search-dirs" | "--print-search-dirs" => {
                    println!("install: /usr/lib/gcc/{}/13/", target.triple());
                    println!("programs: /usr/bin/");
                    println!("libraries: {}", target.implicit_library_paths());
                    return Ok(true);
                }
                _ if arg.starts_with("-print-file-name=") => {
                    let name = &arg["-print-file-name=".len()..];
                    // Special case: "include" should return our bundled include
                    // directory so that build systems (e.g., Linux kernel) pick up
                    // our intrinsic headers (arm_neon.h, emmintrin.h, etc.) instead
                    // of the host GCC's headers which use incompatible builtins.
                    if name == "include" {
                        if let Some(bundled) =
                            crate::frontend::preprocessor::Preprocessor::bundled_include_dir()
                        {
                            println!("{}", bundled.display());
                            return Ok(true);
                        }
                    }
                    // Search standard library directories for the requested file.
                    // If found, print the full path; otherwise echo the name back
                    // (matching GCC behavior).
                    let triple = target.triple();
                    let search_dirs = [
                        format!("/usr/lib/gcc/{}/13/", triple),
                        format!("/usr/lib/gcc-cross/{}/13/", triple),
                        format!("/usr/lib/{}/", triple),
                        format!("/usr/{}/lib/", triple),
                        "/usr/lib/".to_string(),
                    ];
                    let mut found = false;
                    for dir in &search_dirs {
                        let path = format!("{}{}", dir, name);
                        if std::path::Path::new(&path).exists() {
                            println!("{}", path);
                            found = true;
                            break;
                        }
                    }
                    if !found {
                        println!("{}", name);
                    }
                    return Ok(true);
                }
                _ => {}
            }
        }
        Ok(false)
    }

    /// Expand `@file` response file arguments.
    /// Each `@path` argument is replaced by the contents of the file at `path`,
    /// split on whitespace. Non-`@` arguments are passed through unchanged.
    fn expand_response_files(args: &[String]) -> Vec<String> {
        let mut result = Vec::new();
        for arg in args {
            if let Some(path) = arg.strip_prefix('@') {
                if let Ok(contents) = std::fs::read_to_string(path) {
                    // Split on whitespace, respecting simple quoting
                    for token in Self::split_response_file(&contents) {
                        result.push(token);
                    }
                } else {
                    // If the file can't be read, pass the arg through unchanged
                    result.push(arg.clone());
                }
            } else {
                result.push(arg.clone());
            }
        }
        result
    }

    /// Split response file contents into tokens, handling simple quoting.
    fn split_response_file(contents: &str) -> Vec<String> {
        let mut tokens = Vec::new();
        let mut current = String::new();
        let mut in_single_quote = false;
        let mut in_double_quote = false;
        let mut escape = false;

        for ch in contents.chars() {
            if escape {
                current.push(ch);
                escape = false;
                continue;
            }
            match ch {
                '\\' if !in_single_quote => {
                    escape = true;
                }
                '\'' if !in_double_quote => {
                    in_single_quote = !in_single_quote;
                }
                '"' if !in_single_quote => {
                    in_double_quote = !in_double_quote;
                }
                c if c.is_ascii_whitespace() && !in_single_quote && !in_double_quote => {
                    if !current.is_empty() {
                        tokens.push(std::mem::take(&mut current));
                    }
                }
                _ => {
                    current.push(ch);
                }
            }
        }
        if !current.is_empty() {
            tokens.push(current);
        }
        tokens
    }

    /// Parse the main argument list (everything after argv[0]).
    fn parse_main_args(&mut self, args: &[String]) -> Result<(), String> {
        let mut explicit_language: Option<String> = None;
        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                // Output file
                "-o" => {
                    i += 1;
                    if i < args.len() {
                        self.output_path = args[i].clone();
                        self.output_path_set = true;
                    } else {
                        return Err("-o requires an argument".to_string());
                    }
                }

                // Compilation mode flags
                "-S" => self.mode = CompileMode::AssemblyOnly,
                "-c" => self.mode = CompileMode::ObjectOnly,
                "-E" => self.mode = CompileMode::PreprocessOnly,
                "-P" => self.suppress_line_markers = true,
                "-dM" => self.dump_defines = true,

                // Optimization levels.  Numeric values are interpreted by
                // passes::run_passes(): 0=-O0, 1=-O1, 2=-O2, 3=-O3, 4=-Os, 5=-Oz.
                "-O0" => {
                    self.opt_level = 0;
                    self.optimize = false;
                    self.optimize_size = false;
                    self.omit_frame_pointer = false;
                }
                "-O" | "-O1" => {
                    self.opt_level = 1;
                    self.optimize = true;
                    self.optimize_size = false;
                    self.omit_frame_pointer = true;
                }
                "-O2" => {
                    self.opt_level = 2;
                    self.optimize = true;
                    self.optimize_size = false;
                    self.omit_frame_pointer = true;
                }
                "-O3" => {
                    self.opt_level = 3;
                    self.optimize = true;
                    self.optimize_size = false;
                    self.omit_frame_pointer = true;
                }
                "-Os" => {
                    self.opt_level = 4;
                    self.optimize = true;
                    self.optimize_size = true;
                    self.omit_frame_pointer = true;
                }
                "-Oz" => {
                    self.opt_level = 5;
                    self.optimize = true;
                    self.optimize_size = true;
                    self.omit_frame_pointer = true;
                }

                // Debug info
                "-g" => self.debug_info = true,
                arg if arg.starts_with("-g") && arg.len() > 2 => self.debug_info = true,

                // Verbose/diagnostic flags
                "-v" | "--verbose" => self.verbose = true,

                // Linker library flags: -lfoo
                arg if arg.starts_with("-l") => {
                    self.linker_ordered_items.push(arg.to_string());
                }

                // Linker pass-through: -Wl,flag1,flag2,...
                // Keep the whole -Wl argument together so that multi-part flags
                // like -Wl,-soname,libfoo.so and -Wl,-rpath,/path stay intact.
                // The linker code splits on commas internally.
                arg if arg.starts_with("-Wl,") => {
                    self.linker_ordered_items.push(arg.to_string());
                }

                // Linker pass-through: -Xlinker ARG
                // Each -Xlinker passes exactly one argument to the linker.
                // Convert to -Wl,ARG format for uniform downstream handling.
                "-Xlinker" => {
                    i += 1;
                    if i < args.len() {
                        self.linker_ordered_items.push(format!("-Wl,{}", args[i]));
                    }
                }

                // Linker entry point: -e SYMBOL / -eSYMBOL (glibc links libc.so
                // with `-e __libc_main`). Without this the symbol is treated as
                // an input file and the link fails.
                "-e" => {
                    i += 1;
                    if i < args.len() {
                        self.linker_ordered_items
                            .push(format!("-Wl,-e,{}", args[i]));
                    }
                }
                // Export-dynamic family. MUST be matched before the greedy
                // `-e*` entry-point matcher below: `-export-dynamic` would
                // otherwise be swallowed as `-Wl,-e,xport-dynamic`, silently
                // dropping the export request and setting a garbage entry
                // symbol (the binary then exports no dynamic symbols).
                "-export-dynamic" | "--export-dynamic" => {
                    self.linker_ordered_items
                        .push("-Wl,--export-dynamic".to_string());
                }
                arg if arg.len() > 2 && arg.starts_with("-e") && !arg.starts_with("-E") => {
                    self.linker_ordered_items
                        .push(format!("-Wl,-e,{}", &arg[2..]));
                }

                // Assembler pass-through: -Wa,flag1,flag2,...
                arg if arg.starts_with("-Wa,") => {
                    for flag in arg[4..].split(',') {
                        if !flag.is_empty() {
                            self.assembler_extra_args.push(flag.to_string());
                        }
                    }
                }

                // Preprocessor pass-through: -Wp,-MMD,path or -Wp,-MD,path
                arg if arg.starts_with("-Wp,") => {
                    let flags: Vec<&str> = arg[4..].splitn(2, ',').collect();
                    if flags.len() == 2 && (flags[0] == "-MMD" || flags[0] == "-MD") {
                        self.dep_file = Some(flags[1].to_string());
                    }
                }

                // Warning flags
                arg if arg.starts_with("-W") => {
                    let flag = &arg[2..];
                    if !flag.is_empty() {
                        self.warning_config.process_flag(flag);
                    }
                }

                // Preprocessor defines
                "-D" => {
                    i += 1;
                    if i < args.len() {
                        self.add_define(&args[i]);
                    } else {
                        return Err("-D requires an argument".to_string());
                    }
                }
                arg if arg.starts_with("-D") => self.add_define(&arg[2..]),

                // Force-include files
                "-include" => {
                    i += 1;
                    if i < args.len() {
                        self.force_includes.push(args[i].clone());
                    } else {
                        return Err("-include requires an argument".to_string());
                    }
                }

                // Include paths
                "-I" => {
                    i += 1;
                    if i < args.len() {
                        self.add_include_path(&args[i]);
                    } else {
                        return Err("-I requires an argument".to_string());
                    }
                }
                arg if arg.starts_with("-I") => self.add_include_path(&arg[2..]),

                // Quote-only include paths (-iquote)
                "-iquote" => {
                    i += 1;
                    if i < args.len() {
                        self.quote_include_paths.push(args[i].clone());
                    } else {
                        return Err("-iquote requires an argument".to_string());
                    }
                }

                // System include paths (-isystem)
                "-isystem" => {
                    i += 1;
                    if i < args.len() {
                        self.isystem_include_paths.push(args[i].clone());
                    } else {
                        return Err("-isystem requires an argument".to_string());
                    }
                }

                // After include paths (-idirafter)
                "-idirafter" => {
                    i += 1;
                    if i < args.len() {
                        self.after_include_paths.push(args[i].clone());
                    } else {
                        return Err("-idirafter requires an argument".to_string());
                    }
                }

                // Library search paths
                "-L" => {
                    i += 1;
                    if i < args.len() {
                        self.linker_paths.push(args[i].clone());
                    }
                }
                arg if arg.starts_with("-L") => {
                    self.linker_paths.push(arg[2..].to_string());
                }

                // Suppress all predefined macros (-undef)
                // Must come before -U prefix match since -undef starts with -U
                "-undef" => {
                    self.undef_all = true;
                }
                // Undefine macro
                "-U" => {
                    i += 1;
                    if i < args.len() {
                        self.undef_macros.push(args[i].clone());
                    }
                }
                arg if arg.starts_with("-U") => {
                    self.undef_macros.push(arg[2..].to_string());
                }

                // Standard version flag: -std=c99 disables GNU extensions,
                // -std=gnu99 (or no flag) enables them.
                arg if arg == "-fexceptions" => {
                    self.exceptions = true;
                }
                arg if arg == "-fno-exceptions" => {
                    self.exceptions = false;
                }
                // Plain-char signedness overrides (GCC-compatible): these
                // re-record the process-global AFTER the target default was
                // installed, so they win regardless of order relative to it.
                arg if arg == "-funsigned-char" => {
                    crate::common::types::set_char_unsigned(true);
                }
                arg if arg == "-fsigned-char" => {
                    crate::common::types::set_char_unsigned(false);
                }
                // S13: function entry/exit instrumentation via the
                // __cyg_profile_func_enter/exit hooks (GCC contract).
                arg if arg == "-finstrument-functions" => {
                    self.instrument_functions = true;
                }
                arg if arg == "-finstrument-functions-exclude-file-list" => {}
                arg if arg == "-finstrument-functions-exclude-function-list" => {}
                arg if arg.starts_with("-std=") => {
                    let std_value = &arg[5..];
                    // GNU dialects: gnu89, gnu99, gnu11, gnu17, gnu23, etc.
                    // Strict ISO: c89, c99, c11, c17, c23, iso9899:*, etc.
                    self.gnu_extensions = std_value.starts_with("gnu");
                    // gnu89 and c89 use GNU inline semantics by default;
                    // gnu99+ and c99+ use C99 inline semantics.
                    // Note: an EXPLICIT -fgnu89-inline / -fno-gnu89-inline overrides
                    // any -std= (matching GCC: -std= only sets the default model).
                    if self.gnu89_inline_explicit.is_none() {
                        self.gnu89_inline = matches!(
                            std_value,
                            "gnu89" | "c89" | "gnu90" | "c90" | "iso9899:1990" | "iso9899:199409"
                        );
                    }
                }

                // Machine/target flags
                "-mfunction-return=thunk-extern" => self.function_return_thunk = true,
                "-mindirect-branch=thunk-extern" => self.indirect_branch_thunk = true,
                // thunk-inline: full retpoline expanded at each indirect
                // branch site. The kernel vDSO needs this — it is userspace
                // code that cannot reference kernel-internal thunk symbols.
                "-mindirect-branch=thunk-inline" => self.indirect_branch_thunk_inline = true,
                // -mindirect-branch-register: indirect branches only through
                // registers, never memory. Vacuous for lccc: the only
                // indirect-branch patterns emitted are `call *%reg`/`jmp *%reg`.
                "-mindirect-branch-register" => {}
                // -mindirect-branch-cs-prefix: CS segment prefix before
                // indirect-thunk calls so objtool can patch the 6-byte site.
                // Recorded; only meaningful with thunk-extern which we honor.
                "-mindirect-branch-cs-prefix" => self.indirect_branch_cs_prefix = true,
                "-m16" => {
                    // -m16 generates i386 code with .code16gcc prepended so the
                    // GNU assembler adds operand/address-size override prefixes
                    // for 16-bit real mode execution. Used by the Linux kernel
                    // boot code (arch/x86/boot/).
                    self.target = Target::I686;
                    self.code16gcc = true;
                }
                "-m32" => {
                    // Switch to 32-bit i686 target. If already targeting i686
                    // (e.g. invoked as ccc-i686), this is a no-op.
                    if self.target != Target::I686 {
                        self.target = Target::I686;
                    }
                }
                "-m64" => {
                    // 64-bit x86-64 target — the default, but GCC accepts it
                    // explicitly and real build systems pass it (zlib-ng's
                    // configure does). Accept as a no-op when already x86-64.
                    if self.target != Target::X86_64 {
                        self.target = Target::X86_64;
                    }
                }
                "-mno-sse" | "-mno-sse2" => {
                    // Sticky disable: later `-march=native` / CPU profiles must
                    // not revive SSE. GCC keeps `-mno-sse` in either flag order
                    // (kernel decompressor: `-mno-sse` then Cachy `-march=native`).
                    self.no_sse = true;
                    self.sse_explicitly_disabled = true;
                    self.enable_sse3 = false;
                    self.enable_ssse3 = false;
                    self.enable_sse4_1 = false;
                    self.enable_sse4_2 = false;
                    self.enable_avx = false;
                    self.enable_avx2 = false;
                }
                "-mno-mmx" | "-mno-3dnow" => {}
                // ── Kernel -m flags with per-flag semantic justification ──
                // (The blanket -mno-* fallback below covers pure ISA-disable
                // probes; flags listed here need MORE than "we never emit it".)
                //
                // -mno-red-zone: forbid writes below %rsp. lccc NEVER uses the
                // red zone by construction: locals live at non-negative offsets
                // from a frame established by sub-rsp in the prologue, and leaf
                // functions still allocate their frame explicitly.
                "-mno-red-zone" => {}
                // -mno-80387 / -mno-fp-ret-in-387: no x87 instructions, no x87
                // FP returns. NOT vacuous — lccc's long-double path emits x87.
                // Record it so FP codegen can reject long-double operations
                // instead of silently emitting x87 in kernel objects.
                "-mno-80387" | "-msoft-float" | "-mno-fp-ret-in-387" => self.no_x87 = true,
                // -mhard-float: RE-ENABLE hardware FP after a global
                // -msoft-float (kernel CC_FLAGS_FPU for kernel_fpu_begin()
                // translation units, e.g. the NAP governor's nap_fpu.o).
                // lccc's default codegen already uses SSE hardware FP, so
                // this both clears the soft-float latch and is otherwise a
                // no-op. Order matters: kbuild appends it AFTER the kill
                // flags, and later-flag-wins is exactly GCC's semantics.
                "-mhard-float" => self.no_x87 = false,
                // -malign-data=abi|compat|cacheline: data alignment policy.
                // lccc aligns to natural ABI alignment which satisfies abi and
                // compat; cacheline only ever OVER-aligns (a performance
                // preference, not a semantic contract), so accepting is sound.
                s if s.starts_with("-malign-data=") => {}
                // -mharden-sls: straight-line-speculation hardening (INT3
                // after RET/indirect JMP). Not implemented; silently ignoring
                // would strip a requested mitigation.
                s if s.starts_with("-mharden-sls=") && s != "-mharden-sls=none" => {
                    return Err(
                        "ccc: error: -mharden-sls straight-line-speculation hardening \
                                not supported; disable CONFIG_MITIGATION_SLS"
                            .to_string(),
                    );
                }
                "-mharden-sls=none" => {}
                "-mno-avx2" => {
                    self.enable_avx2 = false;
                    self.enable_avxvnni = false;
                    self.enable_avxvnniint8 = false;
                    self.enable_avxvnniint16 = false;
                }
                "-mno-avx" => {
                    self.enable_avx = false;
                    self.enable_avx2 = false;
                    self.enable_avxvnni = false;
                    self.enable_vaes = false;
                    self.enable_vpclmulqdq = false;
                }
                "-mno-sse3" | "-mno-ssse3" | "-mno-sse4" | "-mno-sse4.1" | "-mno-sse4.2" => {
                    self.enable_sse3 = false;
                    self.enable_ssse3 = false;
                    self.enable_sse4_1 = false;
                    self.enable_sse4_2 = false;
                    self.enable_avx = false;
                    self.enable_avx2 = false;
                }
                // Positive SIMD feature flags: define corresponding macros.
                // -mavx2 implies -mavx implies -msse4.2 implies -msse4.1 implies
                // -mssse3 implies -msse3 (matching GCC's implication chain).
                "-maes" => self.enable_aes = true,
                "-mpclmul" => self.enable_pclmul = true,
                "-mf16c" => self.enable_f16c = true,
                "-mfma" => self.enable_fma = true,
                "-mbmi" => self.enable_bmi = true,
                "-mbmi2" => self.enable_bmi2 = true,
                "-mlzcnt" => self.enable_lzcnt = true,
                "-mpopcnt" => self.enable_popcnt = true,
                "-mmovbe" => self.enable_movbe = true,
                "-mrdrnd" => self.enable_rdrnd = true,
                // AVX-512 / AVX10 feature flags (completeness; backend coverage
                // is partial and runtime dispatch must verify the host).
                "-mavx512f" => self.enable_avx512f = true,
                "-mavx512cd" => self.enable_avx512cd = true,
                "-mavx512dq" => self.enable_avx512dq = true,
                "-mavx512bw" => self.enable_avx512bw = true,
                "-mavx512vl" => self.enable_avx512vl = true,
                "-mavx512ifma" => self.enable_avx512ifma = true,
                "-mavx512vbmi" => self.enable_avx512vbmi = true,
                "-mavx512vbmi2" => self.enable_avx512vbmi2 = true,
                "-mavx512vnni" => self.enable_avx512vnni = true,
                "-mavx512bitalg" => self.enable_avx512bitalg = true,
                "-mavx512vpopcntdq" => self.enable_avx512vpopcntdq = true,
                "-mavx512bf16" => self.enable_avx512bf16 = true,
                "-mavx512fp16" => self.enable_avx512fp16 = true,
                "-mavx512er" => self.enable_avx512er = true,
                "-mavx512pf" => self.enable_avx512pf = true,
                "-mavx512vp2intersect" => self.enable_avx512vp2intersect = true,
                "-mavxvnni" => self.enable_avxvnni = true,
                "-mavxifma" => self.enable_avxifma = true,
                "-mavxneconvert" => self.enable_avxneconvert = true,
                "-mavx10.1" | "-mavx10.1-256" | "-mavx10.1-512" | "-mavx10.2" | "-mavx10.2-256"
                | "-mavx10.2-512" => {
                    return Err("AVX10 code generation is not implemented".to_string())
                }
                "-mgfni" => self.enable_gfni = true,
                "-mavxvnniint8" => self.enable_avxvnniint8 = true,
                "-mavxvnniint16" => self.enable_avxvnniint16 = true,
                "-msha512" => self.enable_sha512 = true,
                "-msm3" => self.enable_sm3 = true,
                "-msm4" => self.enable_sm4 = true,
                "-mmovrs" => self.enable_movrs = true,
                "-muser_msr" => self.enable_user_msr = true,
                "-mapxf" => return Err("APX code generation is not implemented".to_string()),
                "-mamx-tile" => self.enable_amx_tile = true,
                "-mamx-int8" => self.enable_amx_int8 = true,
                "-mamx-bf16" => self.enable_amx_bf16 = true,
                "-mcmpccxadd" => self.enable_cmpccxadd = true,
                "-mno-avxvnniint8" => self.enable_avxvnniint8 = false,
                "-mno-avxvnniint16" => self.enable_avxvnniint16 = false,
                "-mno-sha512" => self.enable_sha512 = false,
                "-mno-sm3" => self.enable_sm3 = false,
                "-mno-sm4" => self.enable_sm4 = false,
                "-mno-movrs" => self.enable_movrs = false,
                "-mno-user_msr" => self.enable_user_msr = false,
                "-mno-amx-tile" => self.enable_amx_tile = false,
                "-mno-amx-int8" => self.enable_amx_int8 = false,
                "-mno-amx-bf16" => self.enable_amx_bf16 = false,
                "-mno-cmpccxadd" => self.enable_cmpccxadd = false,
                "-mvaes" => self.enable_vaes = true,
                "-mvpclmulqdq" => self.enable_vpclmulqdq = true,
                "-mno-aes" => self.enable_aes = false,
                "-mno-pclmul" => self.enable_pclmul = false,
                "-mno-f16c" => self.enable_f16c = false,
                "-mno-fma" => self.enable_fma = false,
                "-mno-bmi" => self.enable_bmi = false,
                "-mno-bmi2" => self.enable_bmi2 = false,
                "-mno-lzcnt" => self.enable_lzcnt = false,
                "-mno-popcnt" => self.enable_popcnt = false,
                "-mno-movbe" => self.enable_movbe = false,
                "-mno-rdrnd" => self.enable_rdrnd = false,
                "-mno-avx512f" => self.enable_avx512f = false,
                "-mno-avx512cd" => self.enable_avx512cd = false,
                "-mno-avx512dq" => self.enable_avx512dq = false,
                "-mno-avx512bw" => self.enable_avx512bw = false,
                "-mno-avx512vl" => self.enable_avx512vl = false,
                "-mno-avx512vnni" => self.enable_avx512vnni = false,
                "-mno-avx10.1" => self.enable_avx10_1 = false,
                "-mno-avx10.2" => self.enable_avx10_2 = false,
                "-mno-gfni" => self.enable_gfni = false,
                "-mno-vaes" => self.enable_vaes = false,
                "-mxsave" | "-mxsaveopt" | "-mxsavec" | "-mno-xsave" | "-mno-xsaveopt"
                | "-mno-xsavec" => {}
                "-mno-vpclmulqdq" => self.enable_vpclmulqdq = false,
                "-mavx2" => {
                    // Explicit ISA enable wins over a prior `-mno-sse` (GCC).
                    self.no_sse = false;
                    self.sse_explicitly_disabled = false;
                    self.enable_x86_avx2_profile();
                }
                "-mavx" => {
                    self.no_sse = false;
                    self.sse_explicitly_disabled = false;
                    self.enable_avx = true;
                    self.enable_sse4_2 = true;
                    self.enable_sse4_1 = true;
                    self.enable_ssse3 = true;
                    self.enable_sse3 = true;
                }
                "-msse4.2" => {
                    self.enable_sse4_2 = true;
                    self.enable_sse4_1 = true;
                    self.enable_ssse3 = true;
                    self.enable_sse3 = true;
                }
                "-msse4.1" | "-msse4" => {
                    self.enable_sse4_1 = true;
                    self.enable_ssse3 = true;
                    self.enable_sse3 = true;
                }
                "-mssse3" => {
                    self.enable_ssse3 = true;
                    self.enable_sse3 = true;
                }
                "-msse3" => {
                    self.enable_sse3 = true;
                }
                // Baseline x86-64 ISA flags. SSE2/MMX are the x86-64 default,
                // but `-msse`/`-msse2` after `-mno-sse` must re-enable SSE
                // (GCC last-explicit-ISA-flag wins; `-march=native` does not).
                "-mmmx" => {}
                "-msse2" | "-msse" => {
                    self.no_sse = false;
                    self.sse_explicitly_disabled = false;
                }
                "-m3dnow" => return Err("3DNow! is unsupported".to_string()),
                "-mgeneral-regs-only" => {
                    self.general_regs_only = true;
                    self.no_sse = true;
                    self.sse_explicitly_disabled = true;
                    self.enable_sse3 = false;
                    self.enable_ssse3 = false;
                    self.enable_sse4_1 = false;
                    self.enable_sse4_2 = false;
                    self.enable_avx = false;
                    self.enable_avx2 = false;
                }
                "-mcmodel=kernel" => self.code_model_kernel = true,
                "-mcmodel=small" | "-mcmodel=medlow" | "-mcmodel=medium" | "-mcmodel=medany"
                | "-mcmodel=large" => {
                    self.code_model_kernel = false;
                }
                arg if arg.starts_with("-mabi=") => {
                    self.riscv_abi = Some(arg["-mabi=".len()..].to_string());
                }
                arg if arg.starts_with("-march=") => {
                    let march = &arg["-march=".len()..];
                    if matches!(self.target, Target::X86_64 | Target::I686) {
                        // Remembered for the tuning-row fallback
                        // (cpu_model::resolve: -mtune > -march > generic).
                        self.x86_march = Some(march.to_string());
                    }
                    match self.target {
                        Target::Riscv64 => self.riscv_march = Some(march.to_string()),
                        Target::X86_64 | Target::I686 => match march {
                            "x86-64" | "x86-64-v1" | "generic" => {}
                            // Legacy 32-bit baselines (kernel REALMODE_CFLAGS
                            // pass `-march=i386 -m16`). The i686 backend's
                            // baseline output uses no instruction newer than
                            // the 486 except CMOV in explicit-asm paths; the
                            // kernel pairs this with -mno-sse/-mno-80387 which
                            // are honored independently. Real-mode setup code
                            // executes on the boot CPU — a physical x86-64
                            // machine — so the i686 baseline is sound there.
                            "i386" | "i486" | "i586" | "i686" | "pentium" | "pentiumpro"
                            | "pentium-mmx" => {
                                if self.target != Target::I686 {
                                    return Err(format!(
                                        "-march={} is only valid with -m32/-m16 (i686 target)",
                                        march
                                    ));
                                }
                            }
                            "nehalem" => self.enable_x86_nehalem_profile(),
                            "westmere" => self.enable_x86_westmere_profile(),
                            "sandybridge" => self.enable_x86_sandybridge_profile(),
                            "ivybridge" => self.enable_x86_ivybridge_profile(),
                            "silvermont" => self.enable_x86_silvermont_profile(),
                            "goldmont" => self.enable_x86_goldmont_profile(),
                            "x86-64-v2" => {
                                self.enable_x86_nehalem_profile();
                            }
                            "x86-64-v3" => self.enable_x86_v3_profile(),
                            "haswell" | "broadwell" | "skylake" | "znver1" | "znver2" => {
                                self.enable_x86_haswell_profile()
                            }
                            "znver3" => self.enable_x86_znver3_profile(),
                            "znver4" => self.enable_x86_znver4_profile(),
                            "znver5" => self.enable_x86_znver5_profile(),
                            // Alder Lake ISA (the E-core's capability set: AVX2,
                            // AVX-VNNI, GFNI, VAES, VPCLMULQDQ, no AVX-512):
                            // shared by Raptor Lake, Meteor Lake, Alder Lake-N
                            // and the Gracemont-only spelling.  The *tuning* row
                            // still differs (cpu_model::resolve maps gracemont /
                            // alderlake-n to the E-core row, meteorlake to the
                            // Raptor Lake class).
                            "alderlake" | "raptorlake" | "raptor-lake" | "meteorlake"
                            | "gracemont" | "alderlake-n" => self.enable_x86_alderlake_profile(),
                            // Sierra Forest / Grand Ridge (Crestmont E-core): Alder
                            // Lake ISA + AVX-IFMA, AVX-NE-CONVERT, AVX-VNNI-INT8,
                            // CMPCCXADD — the same additions Arrow Lake carries
                            // (GCC `PTA_SIERRAFOREST`, LLVM `SRFFeatures`).
                            "arrowlake" | "sierraforest" | "grandridge" => {
                                self.enable_x86_arrowlake_profile()
                            }
                            // Clearwater Forest (Darkmont) adds AVX-VNNI-INT16
                            // like Arrow Lake-S / Lunar Lake.
                            "arrowlake-s" | "lunarlake" | "wildcatlake" | "clearwaterforest" => {
                                self.enable_x86_arrowlake_profile();
                                self.enable_avxvnniint16 = true;
                            }
                            "x86-64-v4" => self.enable_x86_avx512_profile(),
                            "skylake-avx512" => self.enable_x86_avx512_cpu_profile(),
                            "cascadelake" => self.enable_x86_cascadelake_profile(),
                            "cooperlake" => self.enable_x86_cooperlake_profile(),
                            "icelake-client" | "icelake-server" | "tigerlake" | "rocketlake" => {
                                self.enable_x86_icelake_profile()
                            }
                            "sapphirerapids" | "graniterapids" | "graniterapids-d" => {
                                self.enable_x86_sapphirerapids_profile()
                            }
                            "knl" => self.enable_x86_knl_profile(),
                            "knm" => {
                                self.enable_x86_knl_profile();
                                self.enable_avx512vpopcntdq = true;
                            }
                            "novalake" | "diamondrapids" => {
                                return Err(format!(
                                    "-march={} requires unimplemented AVX10/APX lowering",
                                    march
                                ))
                            }
                            "native" => {
                                // Detect the HOST CPU's features. Only
                                // meaningful when the compiler itself runs on
                                // x86-64 (cross builds must pass an explicit
                                // profile — matching GCC, whose native probe
                                // also reads the host CPUID).
                                #[cfg(target_arch = "x86_64")]
                                {
                                    if !self.sse_explicitly_disabled {
                                        self.no_sse = false;
                                        if std::arch::is_x86_feature_detected!("sse3") {
                                            self.enable_sse3 = true;
                                        }
                                        if std::arch::is_x86_feature_detected!("ssse3") {
                                            self.enable_ssse3 = true;
                                        }
                                        if std::arch::is_x86_feature_detected!("sse4.1") {
                                            self.enable_sse4_1 = true;
                                        }
                                        if std::arch::is_x86_feature_detected!("sse4.2") {
                                            self.enable_sse4_2 = true;
                                        }
                                        if std::arch::is_x86_feature_detected!("avx") {
                                            self.enable_avx = true;
                                        }
                                        if std::arch::is_x86_feature_detected!("avx2") {
                                            self.enable_avx2 = true;
                                        }
                                        if std::arch::is_x86_feature_detected!("fma") {
                                            self.enable_fma = true;
                                        }
                                    }
                                    if std::arch::is_x86_feature_detected!("bmi1") {
                                        self.enable_bmi = true;
                                    }
                                    if std::arch::is_x86_feature_detected!("bmi2") {
                                        self.enable_bmi2 = true;
                                    }
                                    if std::arch::is_x86_feature_detected!("lzcnt") {
                                        self.enable_lzcnt = true;
                                    }
                                    if std::arch::is_x86_feature_detected!("popcnt") {
                                        self.enable_popcnt = true;
                                    }
                                    if std::arch::is_x86_feature_detected!("movbe") {
                                        self.enable_movbe = true;
                                    }
                                    if !self.sse_explicitly_disabled {
                                        if std::arch::is_x86_feature_detected!("aes") {
                                            self.enable_aes = true;
                                        }
                                        if std::arch::is_x86_feature_detected!("pclmulqdq") {
                                            self.enable_pclmul = true;
                                        }
                                        if std::arch::is_x86_feature_detected!("f16c") {
                                            self.enable_f16c = true;
                                        }
                                        if std::arch::is_x86_feature_detected!("avx512f") {
                                            self.enable_x86_avx512_profile();
                                        }
                                    }
                                }
                                #[cfg(not(target_arch = "x86_64"))]
                                {
                                    return Err("-march=native requires an x86-64 host; pass an explicit profile".to_string());
                                }
                            }
                            _ => return Err(format!("unsupported x86 -march={}", march)),
                        },
                        _ => {
                            return Err(format!(
                                "-march={} is not implemented for target {}",
                                march,
                                self.target.triple()
                            ))
                        }
                    }
                }
                arg if arg.starts_with("-mtune=") => {
                    let tune = &arg["-mtune=".len()..];
                    match self.target {
                        Target::X86_64 | Target::I686 => {
                            // Accept all GCC-compatible tune names (zlib-ng probes
                            // skylake-avx512/cascadelake etc.). Unknown tunes are
                            // stored but otherwise ignored – matching GCC's permissive
                            // handling and ensuring configure scripts never abort.
                            self.x86_tune = Some(tune.to_string());
                        }
                        _ => {
                            return Err(format!(
                                "-mtune={} is not implemented for target {}",
                                tune,
                                self.target.triple()
                            ))
                        }
                    }
                }
                "-mlittle-endian" => {
                    // ARM64 target indicator: only arm64-gcc accepts -mlittle-endian.
                    // This allows `ccc -mlittle-endian` to build ARM code without
                    // requiring the binary to be named aarch64-linux-gnu-ccc.
                    if self.target == Target::X86_64 {
                        self.target = Target::Aarch64;
                    }
                }
                // GCC's -mskip-rax-setup: omit the `xorl %eax,%eax` that tells
                // a variadic callee how many SSE argument registers are live.
                // Legal exactly when no SSE argument register can ever be used,
                // which the kernel guarantees by also passing -mno-sse
                // (arch/x86/Makefile enables both). Honoured only in that
                // combination: dropping the setup while floats can still be
                // passed would leave the callee's register save area undefined.
                "-mskip-rax-setup" => self.skip_rax_setup = true,
                "-mno-relax" => self.riscv_no_relax = true,
                arg if arg.starts_with("-mregparm=") => {
                    let n: u8 = arg["-mregparm=".len()..].parse().unwrap_or(0);
                    self.regparm = n.min(3);
                }
                // `-mpreferred-stack-boundary=N` (GCC) / `-mstack-alignment=N`
                // (Clang): request a stack alignment of 2^N / N bytes.
                //
                // LCCC always keeps %rsp 16-byte aligned at call sites, as the
                // SysV ABI requires. A request for a SMALLER boundary is a
                // permission to align less, not an obligation, so accept it:
                // the kernel passes `-mpreferred-stack-boundary=3` for its
                // realmode and 32-bit entry paths (arch/x86/Makefile). A LARGER
                // boundary is an obligation LCCC cannot meet (no dynamic stack
                // realignment) and must stay an error rather than silently
                // miscompile.
                arg if arg.starts_with("-mpreferred-stack-boundary=")
                    || arg.starts_with("-mstack-alignment=") =>
                {
                    let (val, log2) =
                        if let Some(v) = arg.strip_prefix("-mpreferred-stack-boundary=") {
                            (v, true)
                        } else {
                            (&arg["-mstack-alignment=".len()..], false)
                        };
                    let n: u32 = val
                        .parse()
                        .map_err(|_| format!("invalid argument to {}", arg))?;
                    let bytes = if log2 {
                        1u32.checked_shl(n)
                            .ok_or_else(|| format!("invalid argument to {}", arg))?
                    } else {
                        n
                    };
                    if bytes > 16 {
                        return Err(format!(
                            "{}: LCCC guarantees 16-byte stack alignment and cannot realign the stack to {} bytes",
                            arg, bytes));
                    }
                    // Honour smaller boundaries on i686: the kernel's realmode
                    // code (boundary=2) otherwise pays up to 12 pad bytes per
                    // frame. x86-64 keeps 16 (SSE spill slots + psABI).
                    self.preferred_stack_bytes = bytes.max(4) as u8;
                }
                // Function-entry mcount sub-mode flags. MUST be matched before
                // the `-mno-` and `-m` catch-alls below: `-mnop-mcount` starts
                // with the literal prefix `-mno`, so the permissive disable
                // arm would silently swallow it. Matching GCC's contract, the
                // sub-mode flags are INERT on their own — `-pg` is the trigger
                // that activates instrumentation. The kernel relies on this:
                // `CFLAGS_REMOVE_xxx = -pg` (e.g. arch/x86/entry/vdso) strips
                // `-pg` from objects that can't be instrumented but leaves
                // `-mfentry`/`-mrecord-mcount` in CFLAGS expecting no-ops;
                // activating on `-mfentry` alone would break their link with
                // an undefined `__fentry__`.
                "-mfentry" => {
                    self.mcount_submode.use_fentry = true;
                }
                "-mrecord-mcount" => {
                    self.mcount_submode.record = true;
                }
                "-mnop-mcount" => {
                    self.mcount_submode.nop = true;
                }
                // `-mno-<feature>` for an ISA extension LCCC never emits.
                //
                // Rejecting these is wrong in principle: the flag asks the
                // compiler NOT to use a feature, and a compiler that cannot
                // generate it already complies. The strict policy below exists
                // to catch *enabling* flags whose codegen is unimplemented --
                // silently ignoring those would miscompile. A disable flag has
                // no such hazard. The kernel probes with `-mno-sse4a` and
                // friends (arch/x86/Makefile); a hard error aborted the build
                // at scripts/mod/empty.o before any kernel object compiled.
                // Features LCCC *can* emit keep their explicit arms above.
                arg if arg.starts_with("-mno-") => {
                    if std::env::var("LCCC_STRICT_MFLAGS").is_ok() {
                        return Err(format!(
                            "unsupported machine option {}; LCCC_STRICT_MFLAGS is set",
                            arg
                        ));
                    }
                }
                arg if arg.starts_with("-m") => {
                    return Err(format!("unsupported machine option {}; LCCC refuses to silently ignore target-affecting -m flags", arg));
                }

                // PGO flags (instrumented profiling)
                "-fprofile-generate" => self.pgo_generate = Some(".".to_string()),
                arg if arg.starts_with("-fprofile-generate=") => {
                    let path = arg["-fprofile-generate=".len()..].to_string();
                    self.pgo_generate = Some(if path.is_empty() {
                        ".".to_string()
                    } else {
                        path
                    });
                }
                "-fprofile-use" => self.pgo_use = Some(".".to_string()),
                arg if arg.starts_with("-fprofile-use=") => {
                    let path = arg["-fprofile-use=".len()..].to_string();
                    self.pgo_use = Some(if path.is_empty() {
                        ".".to_string()
                    } else {
                        path
                    });
                }
                "-fprofile-arcs" => self.pgo_generate = Some(".".to_string()),
                "-ftest-coverage" => self.pgo_generate = Some(".".to_string()),
                arg if arg.starts_with("-fprofile-update=") => {
                    // GCC 7+/LLVM 17+ semantics: single (default, non-atomic),
                    // atomic (lock-prefixed), prefer-atomic (atomic when the
                    // target supports it — x86-64 always does).
                    let m = arg["-fprofile-update=".len()..].to_string();
                    self.pgo_update = Some(match m.as_str() {
                        "atomic" | "prefer-atomic" => "atomic".to_string(),
                        _ => "single".to_string(),
                    });
                }
                "-fbranch-probabilities" => {
                    self.pgo_use = Some(self.pgo_use.clone().unwrap_or_else(|| ".".to_string()))
                }
                arg if arg.starts_with("-fprofile-dir=") => {
                    let path = arg["-fprofile-dir=".len()..].to_string();
                    if self.pgo_generate.is_some() {
                        self.pgo_generate = Some(path.clone());
                    }
                    if self.pgo_use.is_some() {
                        self.pgo_use = Some(path);
                    }
                }
                "-fauto-profile" => self.pgo_use = Some(".".to_string()),
                arg if arg.starts_with("-fauto-profile=") => {
                    let path = arg["-fauto-profile=".len()..].to_string();
                    self.pgo_use = Some(path);
                }
                "-fno-profile-arcs"
                | "-fno-test-coverage"
                | "-fno-branch-probabilities"
                | "-fno-auto-profile" => {}
                arg if arg.starts_with("-fno-profile") => {}

                // Feature flags. Full PIC and PIE have different x86-64 data
                // relocation rules, so preserve the last explicit mode rather
                // than collapsing both flags into one boolean.
                "-fPIC" | "-fpic" => {
                    self.pic = true;
                    self.pie = false;
                }
                "-fPIE" | "-fpie" => {
                    self.pic = false;
                    self.pie = true;
                }
                "-fno-PIC" | "-fno-pic" | "-fno-PIE" | "-fno-pie" => {
                    self.pic = false;
                    self.pie = false;
                }
                "-fcf-protection=branch" => {
                    self.cf_protection_branch = true;
                    self.cf_protection_value = Some("1");
                }
                "-fcf-protection=full" | "-fcf-protection" => {
                    self.cf_protection_branch = true;
                    self.cf_protection_value = Some("3");
                }
                "-fcf-protection=return" => {
                    self.cf_protection_branch = false;
                    self.cf_protection_value = Some("2");
                }
                "-fcf-protection=none" => {
                    self.cf_protection_branch = false;
                    self.cf_protection_value = None;
                }
                arg if arg.starts_with("-fpatchable-function-entry=") => {
                    let val = &arg["-fpatchable-function-entry=".len()..];
                    let parts: Vec<&str> = val.split(',').collect();
                    let total: u32 = parts[0].parse().unwrap_or(0);
                    let before: u32 = if parts.len() > 1 {
                        parts[1].parse().unwrap_or(0)
                    } else {
                        0
                    };
                    self.patchable_function_entry = Some((total, before));
                }
                "-fomit-frame-pointer" => self.omit_frame_pointer = true,
                "-fno-omit-frame-pointer" => self.omit_frame_pointer = false,
                "-fno-asynchronous-unwind-tables" | "-fno-unwind-tables" => {
                    self.no_unwind_tables = true
                }
                "-fasynchronous-unwind-tables" | "-funwind-tables" => self.no_unwind_tables = false,
                "-fno-jump-tables" => self.no_jump_tables = true,
                "-ffunction-sections" => self.function_sections = true,
                "-fno-function-sections" => self.function_sections = false,
                "-fdata-sections" => self.data_sections = true,
                "-fno-data-sections" => self.data_sections = false,
                "-fcommon" => self.fcommon = true,
                "-fno-common" => self.fcommon = false,
                "-fgnu89-inline" => {
                    self.gnu89_inline = true;
                    self.gnu89_inline_explicit = Some(true);
                }
                "-fno-gnu89-inline" => {
                    self.gnu89_inline = false;
                    self.gnu89_inline_explicit = Some(false);
                }
                "-fexceptions" => self.exceptions = true,
                "-fno-exceptions" => self.exceptions = false,

                // Floating-point reassociation is an explicit semantic contract,
                // never an implication of -O2/-O3. Packed sum/dot reductions
                // change the source addition order and are only legal when one
                // of these flags permits it. Keep a deliberately narrow bit
                // rather than pretending that every GCC fast-math sub-option is
                // implemented.
                "-ffast-math" => {
                    self.fast_math = true;
                    self.fp_reassoc = true;
                    // GCC: fast-math implies contract=fast only when no
                    // explicit -ffp-contract was given (opts_set semantics).
                    if !self.fp_contract_explicit {
                        self.fp_contract = FpContract::Fast;
                    }
                }
                "-fno-fast-math" => {
                    self.fast_math = false;
                    self.fp_reassoc = false;
                    // Restore the language default. In GCC, -ffp-contract is
                    // orthogonal to -fno-fast-math: the C default (fast)
                    // stands. Explicit -ffp-contract={on,off,fast} wins.
                    if !self.fp_contract_explicit {
                        self.fp_contract = FpContract::c_language_default();
                    }
                }
                "-funsafe-math-optimizations" | "-fassociative-math" => {
                    self.fp_reassoc = true;
                }
                "-fno-unsafe-math-optimizations" | "-fno-associative-math" => {
                    self.fp_reassoc = false;
                }
                "-ffp-contract=fast" => {
                    self.fp_contract = FpContract::Fast;
                    self.fp_contract_explicit = true;
                }
                "-ffp-contract=off" => {
                    self.fp_contract = FpContract::Off;
                    self.fp_contract_explicit = true;
                }
                "-ffp-contract=on" => {
                    // `on` permits language-standard contraction: within ONE
                    // source expression only. The frontend tags every FP
                    // Mul/Add/Sub with its statement root (OP-36), so the
                    // backend fuses `x = a*b + c` but never the split
                    // `t = a*b; s += t;` (GCC `on` semantics).
                    self.fp_contract = FpContract::OnExpr;
                    self.fp_contract_explicit = true;
                }
                // Diagnostic color: -fdiagnostics-color, -fdiagnostics-color={auto,always,never}
                "-fdiagnostics-color" | "-fcolor-diagnostics" => {
                    self.color_mode = ColorMode::Always;
                }
                "-fno-diagnostics-color" | "-fno-color-diagnostics" => {
                    self.color_mode = ColorMode::Never;
                }
                arg if arg.starts_with("-fdiagnostics-color=") => {
                    let value = &arg["-fdiagnostics-color=".len()..];
                    if let Some(mode) = ColorMode::from_flag(value) {
                        self.color_mode = mode;
                    }
                    // Unknown values silently ignored (matching GCC)
                }
                // Function-entry mcount instrumentation: `-pg` is the trigger
                // that activates emission (see McountInstrumentation). The
                // `-m`-prefixed sub-mode flags (-mfentry, -mrecord-mcount,
                // -mnop-mcount) are matched above — before the `-mno-`/`-m`
                // catch-alls — and mutate mcount_submode; they stay inert
                // unless `-pg` activates them, matching the GCC contract the
                // kernel's CFLAGS_REMOVE mechanism depends on.
                "-pg" => {
                    self.mcount_pg = true;
                }
                // Stack-protector request. LCCC does NOT emit canaries, so
                // silently accepting these would hand the caller a binary it
                // believes is hardened but is not -- the worst failure mode for
                // a security feature. `-fno-stack-protector` is what LCCC
                // already does and stays accepted.
                a @ ("-fstack-protector"
                | "-fstack-protector-all"
                | "-fstack-protector-strong"
                | "-fstack-protector-explicit") => {
                    return Err(format!(
                        "{}: LCCC does not implement stack-protector canaries; \
                         build with -fno-stack-protector",
                        a
                    ));
                }
                arg if arg.starts_with("-mstack-protector-guard") => {
                    return Err(format!(
                        "{}: LCCC does not implement stack-protector canaries; \
                         build with -fno-stack-protector",
                        arg
                    ));
                }
                arg if arg.starts_with("-f") => {}

                // Linker flags
                "-static" => self.static_link = true,
                "-shared" => self.shared_lib = true,
                "-r" | "-relocatable" => self.relocatable = true,
                "-no-pie" | "-pie" => {}
                "-nostdlib" => self.nostdlib = true,
                "-nostdinc" => self.nostdinc = true,
                "-nodefaultlibs" => {}

                // Language selection
                "-x" => {
                    i += 1;
                    if i < args.len() {
                        let lang = args[i].as_str();
                        if lang == "none" {
                            explicit_language = None;
                        } else {
                            explicit_language = Some(args[i].clone());
                        }
                    } else {
                        return Err("-x requires an argument".to_string());
                    }
                }

                // Dependency generation flags
                "-MD" | "-MMD" => {
                    if self.dep_file.is_none() {
                        self.dep_file = Some(String::new());
                    }
                }
                "-MP" => {}
                "-M" | "-MM" => {
                    // -M/-MM: dependency-only mode. Preprocess and output
                    // make rules instead of compiling. GCC treats -M/-MM
                    // as implying -E.
                    self.dep_only = true;
                    self.mode = CompileMode::PreprocessOnly;
                }
                "-MF" => {
                    i += 1;
                    if i < args.len() {
                        self.dep_file = Some(args[i].clone());
                    }
                }
                "-MT" | "-MQ" => {
                    i += 1;
                    if i < args.len() {
                        self.dep_target = Some(args[i].clone());
                    }
                }

                // Misc flags
                "-rdynamic" => {
                    self.linker_ordered_items.push("-rdynamic".to_string());
                }
                "-pipe" | "-Xa" | "-Xc" | "-Xt" => {}
                "-pthread" => {
                    self.pthread = true;
                }

                // GCC --param flag: --param <name>=<value> or --param=<name>=<value>
                // Used by nix CC wrapper for hardening flags like ssp-buffer-size=4
                "--param" => {
                    // Skip the next argument (the parameter value)
                    i += 1;
                }
                arg if arg.starts_with("--param=") => {
                    // Single-argument form: --param=ssp-buffer-size=4
                    // Silently ignore
                }

                // Stdin input
                "-" => {
                    self.input_files.push("-".to_string());
                    self.explicit_language = explicit_language.clone();
                }

                // Unknown flags
                arg if arg.starts_with('-') => {
                    if self.verbose {
                        eprintln!("warning: unknown flag: {}", arg);
                    }
                }

                // Input file
                _ => {
                    if explicit_language.is_some() {
                        self.explicit_language = explicit_language.clone();
                    }
                    // Track object/archive files in the ordered linker items list
                    // to preserve their position relative to -l and -Wl, flags.
                    // C source files are compiled to temp objects and placed first.
                    if Self::is_object_or_archive(&args[i]) {
                        self.linker_ordered_items.push(args[i].clone());
                    }
                    self.input_files.push(args[i].clone());
                }
            }
            i += 1;
        }

        Ok(())
    }

    /// Handle -Wl,--version when no input files are given (Meson linker detection).
    ///
    /// When the `gcc_linker` feature is enabled, delegates to GCC for version info.
    /// When disabled, prints built-in linker version info.
    fn run_linker_version_query(target: &Target, linker_items: &[String]) {
        #[cfg(feature = "gcc_linker")]
        {
            let config = target.linker_config();
            let mut cmd = std::process::Command::new(config.command);
            cmd.args(config.extra_args);
            for item in linker_items {
                cmd.arg(item);
            }
            cmd.stdout(std::process::Stdio::inherit());
            cmd.stderr(std::process::Stdio::inherit());
            let _ = cmd.status();
        }
        #[cfg(not(feature = "gcc_linker"))]
        {
            let _ = (target, linker_items);
            // Print GNU ld-compatible version info for build system detection.
            // This is shared with standalone lccc-ld so Kconfig/Meson probes
            // see one stable linker identity on both paths.
            println!("{}", crate::linker_entry::GNU_LD_VERSION_OUTPUT);
        }
    }

    /// Add a -D define from command line.
    pub fn add_define(&mut self, arg: &str) {
        if let Some(eq_pos) = arg.find('=') {
            self.defines.push(CliDefine {
                name: arg[..eq_pos].to_string(),
                value: arg[eq_pos + 1..].to_string(),
            });
        } else {
            self.defines.push(CliDefine {
                name: arg.to_string(),
                value: "1".to_string(),
            });
        }
    }

    /// Add a -I include path from command line.
    pub fn add_include_path(&mut self, path: &str) {
        self.include_paths.push(path.to_string());
    }
}

#[cfg(test)]
mod cli_tests {
    use super::cmp_version;
    use crate::common::fp_contract::FpContract;
    use crate::driver::pipeline::Driver;

    /// Parse one flag against a fresh Driver, returning the error text if any.
    fn try_flag(flag: &str) -> Result<(), String> {
        let mut d = Driver::new();
        // args[0] is argv[0] (used for target detection from the binary name).
        let args = vec!["ccc".to_string(), flag.to_string(), "x.c".to_string()];
        d.parse_cli_args(&args).map(|_| ())
    }

    /// `-mno-<feature>` must be ACCEPTED for any ISA extension LCCC never
    /// emits. A compiler that cannot generate a feature already complies with
    /// a request not to use it, and the Linux kernel probes the compiler with
    /// these before a single object is built (arch/x86/Makefile). Rejecting
    /// them aborted the build at scripts/mod/empty.o.
    #[test]
    fn mno_feature_flags_are_accepted() {
        for f in [
            "-mno-sse4a",
            "-mno-3dnowa",
            "-mno-avx512vbmi2",
            "-mno-tbm",
            "-mno-xop",
            "-mno-fma4",
            "-mno-rtm",
            "-mno-hle",
        ] {
            assert!(try_flag(f).is_ok(), "{} must be accepted", f);
        }
    }

    /// An ENABLING flag whose codegen is unimplemented must still be rejected:
    /// silently ignoring it would miscompile.
    #[test]
    fn unimplemented_enable_flags_still_rejected() {
        assert!(try_flag("-msse4a").is_err(), "-msse4a must be rejected");
    }

    /// Stack alignment: a request for <= 16 bytes is permission to align less
    /// (LCCC always keeps %rsp 16-byte aligned, so it already conforms); a
    /// request for MORE is an obligation LCCC cannot meet and must be an
    /// error rather than a silent miscompile. The kernel passes
    /// `-mpreferred-stack-boundary=3` for realmode/32-bit entry code.
    #[test]
    fn stack_boundary_accepts_smaller_rejects_larger() {
        for f in [
            "-mpreferred-stack-boundary=2",
            "-mpreferred-stack-boundary=3",
            "-mpreferred-stack-boundary=4",
            "-mstack-alignment=8",
            "-mstack-alignment=16",
        ] {
            assert!(try_flag(f).is_ok(), "{} must be accepted", f);
        }
        for f in ["-mpreferred-stack-boundary=5", "-mstack-alignment=32"] {
            assert!(try_flag(f).is_err(), "{} must be rejected", f);
        }
    }

    /// LCCC emits no stack-protector canary. Accepting these silently would
    /// hand the caller a binary it believes is hardened but is not -- the
    /// worst failure mode for a security feature. The kernel then needs
    /// CONFIG_STACKPROTECTOR=n, a decision the build system can only make if
    /// we say so.
    #[test]
    fn unimplemented_hardening_is_refused_not_ignored() {
        for f in [
            "-fstack-protector",
            "-fstack-protector-all",
            "-fstack-protector-strong",
            "-fstack-protector-explicit",
            "-mstack-protector-guard=global",
            "-mstack-protector-guard-reg=gs",
            "-mstack-protector-guard-symbol=__ref_stack_chk_guard",
        ] {
            assert!(
                try_flag(f).is_err(),
                "{} must be refused, not silently ignored",
                f
            );
        }
        // The opposite request is exactly what LCCC already does.
        assert!(try_flag("-fno-stack-protector").is_ok());
    }

    /// The GCC mcount flag family (kernel CONFIG_FUNCTION_TRACER) must parse.
    /// `-pg` is the trigger; the `-m` sub-modes configure the shape and are
    /// inert without it (the kernel's `CFLAGS_REMOVE_x = -pg` VDSO pattern
    /// depends on that). NOTE the ordering hazard under test: `-mnop-mcount`
    /// starts with the literal prefix `-mno` and must not be swallowed by the
    /// permissive disable-flag arm.
    #[test]
    fn mcount_flag_family_parses() {
        for f in ["-pg", "-mfentry", "-mrecord-mcount", "-mnop-mcount"] {
            assert!(try_flag(f).is_ok(), "{} must parse", f);
        }
        let mut d = Driver::new();
        let args: Vec<String> = [
            "ccc",
            "-pg",
            "-mfentry",
            "-mrecord-mcount",
            "-mnop-mcount",
            "x.c",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert!(d.parse_cli_args(&args).is_ok());
        assert!(d.mcount_pg);
        assert_eq!(
            d.mcount_submode,
            crate::backend::McountInstrumentation {
                use_fentry: true,
                record: true,
                nop: true,
            }
        );
        // Sub-modes alone stay inert (no instrumentation without -pg).
        let mut d2 = Driver::new();
        let args2: Vec<String> = ["ccc", "-mfentry", "-mrecord-mcount", "x.c"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(d2.parse_cli_args(&args2).is_ok());
        assert!(!d2.mcount_pg);
    }

    /// `-mskip-rax-setup` is honoured only together with `-mno-sse`, where no
    /// SSE argument register can be live. Both must parse.
    #[test]
    fn skip_rax_setup_parses() {
        assert!(try_flag("-mskip-rax-setup").is_ok());
        let mut d = Driver::new();
        let args: Vec<String> = ["ccc", "-mskip-rax-setup", "-mno-sse", "x.c"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(d.parse_cli_args(&args).is_ok());
    }

    /// Kernel decompressor: `-mno-sse` then Cachy `-march=native`. GCC keeps
    /// the disable; LCCC must not revive SSE via the native CPUID probe.
    #[test]
    fn mno_sse_survives_march_native() {
        let mut d = Driver::new();
        let args: Vec<String> = ["ccc", "-mno-sse", "-march=native", "x.c"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(d.parse_cli_args(&args).is_ok());
        assert!(d.no_sse, "explicit -mno-sse must survive -march=native");
        assert!(d.sse_explicitly_disabled);
        // Reverse order: later -mno-sse still wins.
        let mut d2 = Driver::new();
        let args2: Vec<String> = ["ccc", "-march=native", "-mno-sse", "x.c"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(d2.parse_cli_args(&args2).is_ok());
        assert!(d2.no_sse);
    }

    /// An explicit `-msse` after `-mno-sse` re-enables (GCC last ISA flag).
    #[test]
    fn msse_reenable_after_mno_sse() {
        let mut d = Driver::new();
        let args: Vec<String> = ["ccc", "-mno-sse", "-msse", "x.c"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(d.parse_cli_args(&args).is_ok());
        assert!(!d.no_sse);
        assert!(!d.sse_explicitly_disabled);
    }

    #[test]
    fn fp_reassociation_requires_explicit_flag_and_last_option_wins() {
        let mut strict = Driver::new();
        let strict_args = vec!["lccc".to_string(), "-O3".to_string(), "x.c".to_string()];
        strict.parse_cli_args(&strict_args).unwrap();
        assert!(
            !strict.fp_reassoc,
            "-O3 alone must preserve FP reduction order"
        );

        let mut fast = Driver::new();
        let fast_args = vec![
            "lccc".to_string(),
            "-ffast-math".to_string(),
            "x.c".to_string(),
        ];
        fast.parse_cli_args(&fast_args).unwrap();
        assert!(fast.fp_reassoc);
        assert!(fast.fast_math);

        let mut associative = Driver::new();
        let associative_args = vec![
            "lccc".to_string(),
            "-fassociative-math".to_string(),
            "x.c".to_string(),
        ];
        associative.parse_cli_args(&associative_args).unwrap();
        assert!(associative.fp_reassoc);
        assert!(
            !associative.fast_math,
            "individual flags must not define __FAST_MATH__"
        );

        let mut disabled = Driver::new();
        let disabled_args = vec![
            "lccc".to_string(),
            "-ffast-math".to_string(),
            "-fno-associative-math".to_string(),
            "x.c".to_string(),
        ];
        disabled.parse_cli_args(&disabled_args).unwrap();
        assert!(!disabled.fp_reassoc);
        assert!(
            disabled.fast_math,
            "GCC keeps __FAST_MATH__ for this option sequence"
        );
    }

    #[test]
    fn fp_contract_is_independent_and_last_option_wins() {
        let mut explicit = Driver::new();
        let args = vec![
            "lccc".to_string(),
            "-fassociative-math".to_string(),
            "-ffp-contract=fast".to_string(),
            "x.c".to_string(),
        ];
        explicit.parse_cli_args(&args).unwrap();
        assert!(explicit.fp_reassoc);
        assert!(explicit.fp_contract == FpContract::Fast);

        let mut disabled = Driver::new();
        let args = vec![
            "lccc".to_string(),
            "-ffast-math".to_string(),
            "-ffp-contract=off".to_string(),
            "x.c".to_string(),
        ];
        disabled.parse_cli_args(&args).unwrap();
        assert!(
            disabled.fp_reassoc,
            "contract-off must not disable reassociation"
        );
        assert!(disabled.fp_contract == FpContract::Off);

        let mut reen = Driver::new();
        let args = vec![
            "lccc".to_string(),
            "-ffp-contract=off".to_string(),
            "-ffp-contract=fast".to_string(),
            "x.c".to_string(),
        ];
        reen.parse_cli_args(&args).unwrap();
        assert!(reen.fp_contract == FpContract::Fast);

        // GCC parity: the C default IS -ffp-contract=fast (GNU dialects,
        // since 4.6). The old assertion here ("default must NOT fuse") was
        // refuted by the godbolt oracles: on `t = a[i]*b[i]; s = s + t;`
        // at -O3 -march=x86-64-v3, gcc16.2 and icx emit vfmadd by default
        // -- only clang (default `on`) keeps the separate pair. The
        // backend's FMA3 ISA gate keeps baseline x86-64 builds
        // numerically identical regardless.
        let mut dflt = Driver::new();
        let args = vec!["lccc".to_string(), "x.c".to_string()];
        dflt.parse_cli_args(&args).unwrap();
        assert!(
            dflt.fp_contract == FpContract::Fast,
            "gnu-C default must match GCC: -ffp-contract=fast"
        );

        // Explicit contract flag is sticky against later -ffast-math
        // (GCC opts_set semantics), in BOTH orders.
        let mut sticky = Driver::new();
        let args = vec![
            "lccc".to_string(),
            "-ffp-contract=off".to_string(),
            "-ffast-math".to_string(),
            "x.c".to_string(),
        ];
        sticky.parse_cli_args(&args).unwrap();
        assert!(
            sticky.fp_contract == FpContract::Off,
            "explicit off survives -ffast-math"
        );

        // -fno-fast-math is orthogonal to contraction in GCC: the language
        // default (fast) stands after it.
        let mut nfm = Driver::new();
        let args = vec![
            "lccc".to_string(),
            "-fno-fast-math".to_string(),
            "x.c".to_string(),
        ];
        nfm.parse_cli_args(&args).unwrap();
        assert!(
            nfm.fp_contract == FpContract::Fast,
            "-fno-fast-math restores the language default (GCC orthogonality)"
        );

        // ... and an explicit off still survives -fno-fast-math.
        let mut nfm_off = Driver::new();
        let args = vec![
            "lccc".to_string(),
            "-ffp-contract=off".to_string(),
            "-fno-fast-math".to_string(),
            "x.c".to_string(),
        ];
        nfm_off.parse_cli_args(&args).unwrap();
        assert!(nfm_off.fp_contract == FpContract::Off);
    }

    #[test]
    fn version_compare_is_numeric() {
        // Lexical sort would order "9.5.0" after "16.1.1"; numeric must not.
        assert!(cmp_version("16.1.1", "9.5.0") == std::cmp::Ordering::Greater);
        assert!(cmp_version("12", "9.5.0") == std::cmp::Ordering::Greater);
        assert!(cmp_version("16.1.1", "16.1.1") == std::cmp::Ordering::Equal);
        assert!(cmp_version("16.2.0", "16.1.1") == std::cmp::Ordering::Greater);
        assert!(cmp_version("10.2.0", "11.4.0") == std::cmp::Ordering::Less);
        assert!(cmp_version("4.9", "4.10") == std::cmp::Ordering::Less);
        // Newest-first sort used by -print-libgcc-file-name.
        let mut v = vec![
            "10.2.0".to_string(),
            "9.5.0".to_string(),
            "16.1.1".to_string(),
            "12".to_string(),
        ];
        v.sort_by(|a, b| cmp_version(b, a));
        assert_eq!(v, vec!["16.1.1", "12", "10.2.0", "9.5.0"]);
    }

    #[test]
    fn export_dynamic_is_not_swallowed_by_entry_matcher() {
        let mut d = Driver::new();
        let args: Vec<String> = ["lccc", "-export-dynamic", "x.c"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        d.parse_cli_args(&args).ok();
        assert!(
            d.linker_ordered_items
                .iter()
                .any(|i| i == "-Wl,--export-dynamic"),
            "linker items: {:?}",
            d.linker_ordered_items
        );
        assert!(
            !d.linker_ordered_items
                .iter()
                .any(|i| i.starts_with("-Wl,-e,")),
            "must not be parsed as an entry-point flag: {:?}",
            d.linker_ordered_items
        );
        // --export-dynamic long form too.
        let mut d2 = Driver::new();
        let args2: Vec<String> = ["lccc", "--export-dynamic", "x.c"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        d2.parse_cli_args(&args2).ok();
        assert!(d2
            .linker_ordered_items
            .iter()
            .any(|i| i == "-Wl,--export-dynamic"));
    }

    #[test]
    fn entry_flag_still_works() {
        // -e SYMBOL must keep mapping to -Wl,-e,SYMBOL.
        let mut d = Driver::new();
        let args: Vec<String> = ["lccc", "-e", "__libc_main", "x.c"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        d.parse_cli_args(&args).ok();
        assert!(d
            .linker_ordered_items
            .iter()
            .any(|i| i == "-Wl,-e,__libc_main"));
        // -eSYMBOL compact form.
        let mut d2 = Driver::new();
        let args2: Vec<String> = ["lccc", "-emain", "x.c"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        d2.parse_cli_args(&args2).ok();
        assert!(d2.linker_ordered_items.iter().any(|i| i == "-Wl,-e,main"));
    }

    /// `-mno-sse -march=haswell` must not set AVX2: the 64-byte memcpy path
    /// keys off `avx2_enabled` and would otherwise emit `vmovdqu`.
    #[test]
    fn mno_sse_survives_march_haswell_without_avx() {
        let mut d = Driver::new();
        let args: Vec<String> = ["ccc", "-mno-sse", "-march=haswell", "x.c"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(d.parse_cli_args(&args).is_ok());
        assert!(d.no_sse);
        assert!(!d.enable_avx);
        assert!(!d.enable_avx2);
        assert!(d.enable_bmi, "integer BMI remains legal under -mno-sse");
    }

    #[test]
    fn mavx_reenable_after_mno_sse() {
        let mut d = Driver::new();
        let args: Vec<String> = ["ccc", "-mno-sse", "-mavx", "x.c"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(d.parse_cli_args(&args).is_ok());
        assert!(!d.no_sse);
        assert!(d.enable_avx);
    }
}
