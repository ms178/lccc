pub(crate) mod asm_expr; // Shared assembly expression evaluator (arithmetic, bitwise, parens)
pub(crate) mod asm_preprocess; // Shared GAS preprocessing: comments, macros, rept, conditionals
pub(crate) mod common;
#[allow(dead_code)] // Defines ELF standard constants/helpers; not all used by every backend
pub(crate) mod elf;
pub(crate) mod elf_writer_common; // Shared x86/i686 assembler ELF writer
#[cfg_attr(feature = "gcc_linker", allow(dead_code))]
// Built-in linker code unused when gcc handles linking
pub mod linker_common;
pub(crate) mod peephole_common; // Shared peephole optimizer utilities (word matching, LineStore)

// Shared codegen framework, split into focused modules:
pub(crate) mod call_abi; // Unified ABI classification: call args + callee params, stack computation
pub(crate) mod cast; // Cast and float operation classification
pub(crate) mod f128_softfloat; // Shared F128 soft-float orchestration (ARM + RISC-V)
pub(crate) mod generation; // Module/function/instruction dispatch
pub(crate) mod inline_asm; // InlineAsmEmitter trait and shared framework
pub(crate) mod stack_layout; // Stack layout: slot assignment, alloca coalescing, regalloc helpers
pub(crate) mod state; // CodegenState, StackSlot, SlotAddr
pub(crate) mod traits; // ArchCodegen trait with default implementations
pub(crate) mod x86_common; // Shared x86/i686 register names, condition codes, asm template parsing

// Register allocation and liveness analysis
pub(crate) mod live_range; // Linear scan data structures (LiveRange, LinearScanAllocator)
pub(crate) mod liveness; // Live interval computation
pub(crate) mod regalloc; // Linear scan register allocator
pub(crate) mod split_ranges; // Live range splitting for call-spanning values

pub(crate) mod arm;
pub(crate) mod i686;
pub(crate) mod riscv;
pub mod x86;

use crate::ir::reexports::IrModule;

/// Function-entry mcount instrumentation, derived from `-pg` (and its `-m`
/// sub-mode flags).
///
/// GCC's mcount flag family is implemented at the codegen level by emitting a
/// single 5-byte instruction (a `call` or a NOP) at function entry, optionally
/// recorded in `__mcount_loc` for the runtime patcher. Measured GCC 14.2
/// reference shapes:
///
/// - `-pg` alone: frame is set up FIRST (`push %rbp; mov %rsp,%rbp`), then
///   `call mcount` — the classic mcount ABI reads the parent PC through the
///   frame. GCC rejects `-pg` together with `-fomit-frame-pointer`.
/// - `-pg -mfentry`: `call __fentry__` is the very FIRST instruction (before
///   any prologue save) — the kernel x86_64 default.
/// - `-pg -mrecord-mcount`: also emit a pointer-sized entry in a
///   `__mcount_loc,"a",@progbits` section pointing at each call site
///   (`CONFIG_FTRACE_MCOUNT_USE_CC=y`; no post-link recordmcount step).
/// - `-pg -mnop-mcount`: replace the call with the kernel's canonical 5-byte
///   NOP (`0f 1f 44 00 00`); the location is still recorded when `record` is
///   also set. With `CONFIG_HAVE_OBJTOOL_NOP_MCOUNT` objtool converts the
///   recorded sites at link time.
///
/// Contract: `-pg` is the trigger; the `-m` sub-mode flags are inert without
/// it. The kernel relies on this — `CFLAGS_REMOVE_xxx = -pg` (e.g. for VDSO
/// objects) leaves `-mfentry`/`-mrecord-mcount` in CFLAGS expecting no-ops.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct McountInstrumentation {
    /// `-mfentry`: emit `call __fentry__` instead of `call mcount`.
    pub use_fentry: bool,
    /// `-mrecord-mcount`: record each call site in `__mcount_loc`.
    pub record: bool,
    /// `-mnop-mcount`: emit a 5-byte NOP instead of a call. The location is
    /// still recorded in `__mcount_loc` when `record` is also set.
    pub nop: bool,
}

/// Options that control code generation, parsed from CLI flags.
#[derive(Debug, Clone, Default)]
pub(crate) struct CodegenOptions {
    /// Disable register allocation for non-SSA `-O0` IR. Phi elimination can
    /// create multi-def values that the production scan must not coalesce.
    pub(crate) disable_regalloc: bool,
    /// Whether to generate fully interposable position-independent code
    /// (`-fPIC`/`-fpic`, or a shared object).
    pub(crate) pic: bool,
    /// Whether to generate position-independent executable code (`-fPIE`/
    /// `-fpie`). Unlike full PIC, ordinary data definitions in the executable
    /// are non-preemptible and may use direct RIP-relative references.
    pub(crate) pie: bool,
    /// Whether to replace `ret` with `jmp __x86_return_thunk` (-mfunction-return=thunk-extern)
    pub(crate) function_return_thunk: bool,
    /// Whether to replace indirect calls/jumps with retpoline thunks (-mindirect-branch=thunk-extern)
    pub(crate) indirect_branch_thunk: bool,
    /// Whether to expand the full retpoline inline at each indirect branch
    /// site (-mindirect-branch=thunk-inline). Kernel vDSO objects use this:
    /// they run in userspace and cannot reference kernel thunk symbols.
    pub(crate) indirect_branch_thunk_inline: bool,
    /// Patchable function entry: (total_nops, nops_before_entry).
    /// -fpatchable-function-entry=N[,M] emits NOP padding around function entry points
    /// and records them in __patchable_function_entries for runtime patching (ftrace).
    pub(crate) patchable_function_entry: Option<(u32, u32)>,
    /// Function-entry mcount instrumentation: -pg / -mfentry / -mrecord-mcount /
    /// -mnop-mcount. See `McountInstrumentation` for the measured GCC contract.
    pub(crate) mcount: Option<McountInstrumentation>,
    /// Whether to emit endbr64 at function entry points (-fcf-protection=branch).
    /// Required for Intel CET/IBT (Indirect Branch Tracking).
    pub(crate) cf_protection_branch: bool,
    /// Whether SSE is disabled (-mno-sse). When true, the x86 codegen avoids
    /// SSE/XMM instructions in variadic prologues (XMM register saving) and
    /// va_start sets fp_offset to overflow so va_arg never uses XMM regs.
    /// TODO: Full -mno-sse support would also need to avoid SSE in float
    /// operations, casts, and other FP codegen paths. Currently only the
    /// variadic ABI path is gated, which is sufficient for the Linux kernel.
    /// Byte alignment for function entry labels (`.p2align log2`). 0 = none.
    /// GCC/Clang use 16 at -O1..-O3 and none at -Os/-Oz.
    pub(crate) function_alignment: u32,
    /// -mskip-rax-setup: omit the `xorl %eax,%eax` / `movb $n,%al` reporting the
    /// number of live SSE argument registers to a variadic callee. Only honoured
    /// together with `no_sse`, where no such register can be in use.
    pub(crate) skip_rax_setup: bool,
    pub(crate) no_sse: bool,
    /// Whether to use only general-purpose registers (-mgeneral-regs-only).
    /// On AArch64, this prevents FP/SIMD register usage in variadic function
    /// prologues (no q0-q7 saves) and sets __vr_offs=0 in va_start.
    /// The Linux kernel uses this to avoid touching NEON/FP state.
    /// TODO: Full -mgeneral-regs-only support would also need to avoid NEON/FP in
    /// popcount, byte-swap, float casts, and other FP codegen paths. Currently only
    /// the variadic ABI path is gated, which is sufficient for the Linux kernel
    /// (kernel code doesn't use floats or popcount builtins in hot paths).
    pub(crate) general_regs_only: bool,
    /// Whether to use the kernel code model (-mcmodel=kernel). All symbols
    /// are assumed to be in the negative 2GB of the virtual address space.
    /// Uses absolute sign-extended 32-bit addressing (movq $symbol) for
    /// global address references, producing R_X86_64_32S relocations.
    pub(crate) code_model_kernel: bool,
    /// Whether to disable jump table emission for switch statements (-fno-jump-tables).
    /// When true, all switch statements use compare-and-branch chains instead of
    /// indirect jumps through a jump table. Required by the Linux kernel when building
    /// with retpoline (-mindirect-branch=thunk-extern) to avoid indirect jumps that
    /// objtool would reject.
    pub(crate) no_jump_tables: bool,
    /// Whether the target has BMI2 (`-mbmi2` or an enabling `-march`).
    /// Gates SHLX/SHRX/SARX selection for variable shifts.
    pub(crate) bmi2: bool,
    /// Measured microarchitectural tuning row selected by `-mtune`/`-march`
    /// (see `backend::x86::cpu_model`).  Drives every decision that depends
    /// on instruction latency/µop facts rather than ISA availability.
    pub(crate) tune: crate::backend::x86::cpu_model::X86Tune,
    /// Whether the target has BMI1 (`-mbmi` or an enabling `-march`).
    /// Gates scalar ANDN selection; emitting it without this contract would
    /// introduce SIGILL on baseline x86-64.
    pub(crate) bmi1: bool,
    /// Whether the target has LZCNT (`-mlzcnt`, ABM, or an enabling
    /// `-march` such as x86-64-v3). LZCNT/TZCNT are NOT baseline x86-64:
    /// on CPUs without the feature the F3-prefixed encodings (F3 0F BD/BC)
    /// decode as BSR/BSF and silently return the bit INDEX instead of a
    /// zero COUNT — no #UD, just wrong data (observed as the preboot ZSTD
    /// decoder's "corruption detected" when an lccc-built kernel boots on
    /// QEMU's default qemu64 TCG CPU). Gates the Clz/Ctz lowering; the
    /// fallback is BSR/BSF plus an explicit zero fixup so the IR's defined
    /// Clz(0)/Ctz(0) == width semantics (matching constant folding) hold.
    pub(crate) lzcnt: bool,
    /// Whether the target has POPCNT (`-mpopcnt` or an enabling `-march`
    /// such as x86-64-v2/Nehalem). 0F B8 is #UD on pre-Nehalem x86-64.
    /// Gates the Popcount lowering; the fallback is an shr/adc bit loop
    /// in %rax/%rcx.
    pub(crate) popcnt: bool,
    /// Whether the target has AVX2 (`-mavx2` or an enabling `-march`).
    pub(crate) avx2: bool,
    /// Whether the target has AVX-512F (from -mavx512f / -march=*avx512*).
    /// Enables the 1-uop EVEX GPR-source vpbroadcast for scalar->vector splats.
    pub(crate) avx512: bool,
    /// Whether to suppress linker relaxation (-mno-relax, RISC-V only).
    /// When true, the codegen emits `.option norelax` at the top of the
    /// assembly output, which prevents the GNU assembler from generating
    /// R_RISCV_RELAX relocation entries. This is required for the Linux
    /// kernel's EFI stub, which uses -fpic -mno-relax to ensure no
    /// absolute symbol references are introduced by linker relaxation.
    pub(crate) no_relax: bool,
    /// Whether to emit debug info (.file/.loc directives) when compiling with -g.
    /// When true, the codegen emits DWARF line number directives based on
    /// source_spans attached to each IR instruction during lowering.
    pub(crate) debug_info: bool,
    /// Whether to place each function in its own ELF section (-ffunction-sections).
    /// When true, each function is emitted into `.text.funcname` instead of `.text`.
    /// This enables the linker's `--gc-sections` to discard unreferenced functions.
    pub(crate) function_sections: bool,
    /// Whether to place each data object in its own ELF section (-fdata-sections).
    /// When true, each global variable is emitted into its own section
    /// (e.g., `.data.varname`, `.rodata.varname`, `.bss.varname`).
    /// This enables the linker's `--gc-sections` to discard unreferenced data.
    pub(crate) data_sections: bool,
    /// Whether to prepend `.code16gcc` to the assembly output (-m16).
    /// When true, the GNU assembler treats the 32-bit instructions as code
    /// that will run in 16-bit real mode, adding operand/address-size override
    /// prefixes as needed. Used by the Linux kernel boot code.
    pub(crate) code16gcc: bool,
    /// Number of integer arguments passed in registers (i686 only, -mregparm=N).
    /// 0 = standard cdecl (all args on stack), 1-3 = pass first N integer args
    /// in EAX, EDX, ECX respectively. Used by the Linux kernel boot code
    /// (-mregparm=3) to reduce code size in 16-bit real mode.
    pub(crate) regparm: u8,
    /// Requested stack boundary in bytes (16 = SysV default). i686 honours
    /// 4/8 to shrink frames in kernel realmode code; x86-64 ignores <16.
    pub(crate) preferred_stack_bytes: u8,
    /// Permit fused multiply-add contraction in BACKEND fusion (scalar
    /// vfmadd231s{s,d} on x86, fmadd on AArch64). The vectorizer receives the
    /// same contract via run_passes; both consumers must agree or
    /// -ffp-contract=off silently loses its single-rounding guarantee in
    /// whichever layer was forgotten.
    pub(crate) fp_contract: crate::common::fp_contract::FpContract,
    /// Whether the x86 target has FMA3 (`-mfma` or an enabling `-march` such
    /// as x86-64-v3). Scalar FMA fusion (`vfmadd231s{s,d}`) is contraction
    /// *and* an ISA feature: GCC only emits it when both are present, and
    /// emitting FMA3 on a baseline (SSE2) target is SIGILL on pre-Haswell
    /// hardware. AArch64 needs no such gate (`fmadd` is baseline ISA there);
    /// i686/RISC-V never emit fused FP mul-add.
    pub(crate) fma: bool,
    /// Whether to omit the frame pointer (-fomit-frame-pointer).
    /// When true, functions do not set up EBP as a frame pointer, freeing it
    /// as a general register and saving prologue/epilogue instructions.
    /// Used by the Linux kernel boot code to reduce code size.
    pub(crate) omit_frame_pointer: bool,
    /// Whether to emit CFI directives (.cfi_startproc, .cfi_endproc, etc.)
    /// for generating .eh_frame unwind tables. Enabled by default (like GCC).
    /// Disabled by -fno-asynchronous-unwind-tables or -fno-unwind-tables.
    /// Many programs (LuaJIT, libunwind users) require .eh_frame for exception
    /// handling and stack unwinding.
    pub(crate) emit_cfi: bool,
    /// Whether to optimize for size (-Os/-Oz). When true, codegen prefers the
    /// shorter sequence over the faster one (e.g. `idiv` over a magic-number
    /// multiply for constant division, `imul` over a multi-instruction LEA
    /// chain). When false (-O1/-O2/-O3), codegen uses the fastest sequence
    /// even if it is a few bytes longer. Mirrors GCC/Clang's -Os behaviour.
    pub(crate) optimize_for_size: bool,
}

/// Target architecture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    X86_64,
    I686,
    Aarch64,
    Riscv64,
}

impl Target {
    /// Return the GCC-style target triple for this architecture.
    /// Used by configure scripts (via -dumpmachine) to detect the target.
    pub fn triple(&self) -> &'static str {
        match self {
            Target::X86_64 => "x86_64-linux-gnu",
            Target::I686 => "i686-linux-gnu",
            Target::Aarch64 => "aarch64-linux-gnu",
            Target::Riscv64 => "riscv64-linux-gnu",
        }
    }

    /// Return the dynamic linker path for this target.
    pub(crate) fn dynamic_linker(&self) -> &'static str {
        match self {
            Target::X86_64 => "/lib64/ld-linux-x86-64.so.2",
            Target::I686 => "/lib/ld-linux.so.2",
            Target::Aarch64 => "/lib/ld-linux-aarch64.so.1",
            Target::Riscv64 => "/lib/ld-linux-riscv64-lp64d.so.1",
        }
    }

    /// Return the implicit library search directories for this target.
    /// This is used by the driver to emit `LIBRARY_PATH=...` during verbose
    /// linking, which CMake parses to discover implicit link directories
    /// (needed for `find_library()` to locate libraries like libm in
    /// multiarch paths like /usr/lib/x86_64-linux-gnu/).
    pub(crate) fn implicit_library_paths(&self) -> String {
        let triple = self.triple();

        // GCC lib base paths and versions to probe
        let gcc_bases: &[&str] = match self {
            Target::X86_64 => &[
                "/usr/lib/gcc/x86_64-linux-gnu",
                "/usr/lib/gcc/x86_64-redhat-linux",
                "/usr/lib64/gcc/x86_64-linux-gnu",
            ],
            Target::I686 => &[
                "/usr/lib/gcc-cross/i686-linux-gnu",
                "/usr/lib/gcc/i686-linux-gnu",
                "/usr/lib/gcc/i386-linux-gnu",
            ],
            Target::Aarch64 => &[
                "/usr/lib/gcc-cross/aarch64-linux-gnu",
                "/usr/lib/gcc/aarch64-linux-gnu",
            ],
            Target::Riscv64 => &[
                "/usr/lib/gcc-cross/riscv64-linux-gnu",
                "/usr/lib/gcc/riscv64-linux-gnu",
            ],
        };
        let gcc_versions: &[&str] = &["14", "13", "12", "11", "10", "9", "8", "7"];

        let mut paths: Vec<String> = Vec::new();

        // Find GCC lib dir (contains crtbegin.o)
        'outer: for base in gcc_bases {
            for ver in gcc_versions {
                let dir = format!("{}/{}", base, ver);
                if std::path::Path::new(&format!("{}/crtbegin.o", dir)).exists() {
                    paths.push(dir);
                    break 'outer;
                }
            }
        }

        // Multiarch lib dirs
        let lib_dir = format!("/usr/lib/{}", triple);
        if std::path::Path::new(&lib_dir).exists() {
            paths.push(lib_dir);
        }
        let lib_alt = format!("/lib/{}", triple);
        if std::path::Path::new(&lib_alt).exists() {
            paths.push(lib_alt);
        }

        // Cross-compiler lib dirs
        let cross_lib = format!("/usr/{}/lib", triple);
        if std::path::Path::new(&cross_lib).exists() {
            paths.push(cross_lib);
        }

        // Generic fallback dirs
        for dir in &["/usr/lib", "/lib"] {
            if std::path::Path::new(dir).exists() {
                paths.push(dir.to_string());
            }
        }

        paths.join(":")
    }

    /// Whether this target uses 32-bit pointers (ILP32 data model).
    pub(crate) fn is_32bit(&self) -> bool {
        matches!(self, Target::I686)
    }

    /// Pointer size in bytes for this target.
    pub(crate) fn ptr_size(&self) -> usize {
        if self.is_32bit() {
            4
        } else {
            8
        }
    }

    /// ELF e_machine value for this target. Relocation-type number spaces are
    /// per ISA — classification of a reloc (e.g. TLS vs PLT call) must key off
    /// the machine, never off the pointer size (R_RISCV_CALL_PLT=19 collides
    /// with x86-64 R_X86_64_TLSGD=19).
    pub(crate) fn elf_machine(&self) -> u16 {
        match self {
            Target::I686 => elf::EM_386,
            Target::X86_64 => elf::EM_X86_64,
            Target::Aarch64 => elf::EM_AARCH64,
            Target::Riscv64 => elf::EM_RISCV,
        }
    }

    /// Get the assembler config for this target.
    /// Only used when the `gcc_assembler` feature is enabled for GCC fallback.
    #[cfg_attr(not(feature = "gcc_assembler"), allow(dead_code))]
    pub(crate) fn assembler_config(&self) -> common::AssemblerConfig {
        match self {
            Target::X86_64 => common::AssemblerConfig {
                command: "gcc",
                extra_args: &[],
            },
            Target::I686 => common::AssemblerConfig {
                command: "i686-linux-gnu-gcc",
                extra_args: &["-m32"],
            },
            Target::Aarch64 => common::AssemblerConfig {
                command: "aarch64-linux-gnu-gcc",
                extra_args: &["-march=armv8-a+crc+crypto"],
            },
            Target::Riscv64 => common::AssemblerConfig {
                command: "riscv64-linux-gnu-gcc",
                extra_args: &["-march=rv64gc", "-mabi=lp64d"],
            },
        }
    }

    /// Get the linker config for this target.
    pub(crate) fn linker_config(&self) -> common::LinkerConfig {
        // ELF e_machine constants (from elf.h):
        // EM_386 = 3, EM_AARCH64 = 183, EM_X86_64 = 62, EM_RISCV = 243
        match self {
            Target::X86_64 => common::LinkerConfig {
                command: "gcc",
                extra_args: &["-no-pie"],
                expected_elf_machine: 62, // EM_X86_64
                arch_name: "x86-64",
            },
            Target::I686 => common::LinkerConfig {
                command: "i686-linux-gnu-gcc",
                extra_args: &["-m32", "-no-pie"],
                expected_elf_machine: 3, // EM_386
                arch_name: "i686",
            },
            Target::Aarch64 => common::LinkerConfig {
                command: "aarch64-linux-gnu-gcc",
                // Use -no-pie to match non-PIC code generation.  The previous
                // default of -static prevented dlopen() of shared libraries
                // at runtime, breaking postgres extension loading.  The unit
                // test harness passes -static explicitly for QEMU user-mode.
                extra_args: &["-no-pie"],
                expected_elf_machine: 183, // EM_AARCH64
                arch_name: "aarch64",
            },
            Target::Riscv64 => common::LinkerConfig {
                command: "riscv64-linux-gnu-gcc",
                extra_args: &["-no-pie"],
                expected_elf_machine: 243, // EM_RISCV
                arch_name: "riscv64",
            },
        }
    }

    /// Generate assembly with full codegen options and optional source manager for debug info.
    /// When `source_mgr` is provided and `opts.debug_info` is true, the codegen emits
    /// .file/.loc directives for DWARF line number information.
    pub(crate) fn generate_assembly_with_opts_and_debug(
        &self,
        module: &IrModule,
        opts: &CodegenOptions,
        source_mgr: Option<&crate::common::source::SourceManager>,
    ) -> String {
        match self {
            Target::X86_64 => {
                let mut cg = x86::X86Codegen::new();
                cg.apply_options(opts);
                cg.state.fpo_requested = opts.omit_frame_pointer;
                cg.state.function_sections = opts.function_sections;
                cg.state.data_sections = opts.data_sections;
                let raw = generation::generate_module_with_debug(
                    &mut cg,
                    module,
                    opts.debug_info,
                    source_mgr,
                );
                // Escape hatch for bisecting a suspected peephole miscompile:
                // LCCC_NO_PEEPHOLE=1 emits the pre-peephole assembly verbatim.
                if std::env::var_os("LCCC_NO_PEEPHOLE").is_some() {
                    raw
                } else {
                    x86::codegen::peephole::peephole_optimize(raw)
                }
            }
            Target::I686 => {
                let mut cg = i686::I686Codegen::new();
                cg.apply_options(opts);
                cg.state.fpo_requested = opts.omit_frame_pointer;
                cg.state.function_sections = opts.function_sections;
                cg.state.data_sections = opts.data_sections;
                let raw = generation::generate_module_with_debug(
                    &mut cg,
                    module,
                    opts.debug_info,
                    source_mgr,
                );
                // Same bisection escape hatch as x86-64: LCCC_NO_PEEPHOLE=1
                // emits the pre-peephole assembly verbatim. Without this the
                // i686 peepholes could not be ruled out when triaging a
                // miscompile (the flag silently did nothing on -m32).
                let optimized = if std::env::var_os("LCCC_NO_PEEPHOLE").is_some() {
                    raw
                } else {
                    i686::codegen::peephole::peephole_optimize(raw)
                };
                if opts.code16gcc {
                    format!(".code16gcc\n{}", optimized)
                } else {
                    optimized
                }
            }
            Target::Aarch64 => {
                let mut cg = arm::ArmCodegen::new();
                cg.apply_options(opts);
                cg.state.fpo_requested = opts.omit_frame_pointer;
                cg.state.function_sections = opts.function_sections;
                cg.state.data_sections = opts.data_sections;
                let raw = generation::generate_module_with_debug(
                    &mut cg,
                    module,
                    opts.debug_info,
                    source_mgr,
                );
                arm::codegen::peephole::peephole_optimize(raw)
            }
            Target::Riscv64 => {
                let mut cg = riscv::RiscvCodegen::new();
                cg.apply_options(opts);
                cg.state.fpo_requested = opts.omit_frame_pointer;
                cg.state.function_sections = opts.function_sections;
                cg.state.data_sections = opts.data_sections;
                cg.emit_pre_directives();
                let raw = generation::generate_module_with_debug(
                    &mut cg,
                    module,
                    opts.debug_info,
                    source_mgr,
                );
                riscv::codegen::peephole::peephole_optimize(raw)
            }
        }
    }

    /// Assemble text to object file with dynamic extra arguments.
    /// Used to pass through -mabi= and -march= flags from the CLI.
    ///
    /// When the `gcc_assembler` Cargo feature is enabled, uses GCC for assembling
    /// (with a warning). When disabled (default), uses the built-in assembler.
    pub(crate) fn assemble_with_extra(
        &self,
        asm_text: &str,
        output_path: &str,
        extra_args: &[String],
    ) -> Result<(), String> {
        // When gcc_assembler feature is enabled, use GCC for assembling
        #[cfg(feature = "gcc_assembler")]
        {
            common::assemble_with_extra(&self.assembler_config(), asm_text, output_path, extra_args)
        }

        // Default (gcc_assembler disabled): use the built-in assembler
        #[cfg(not(feature = "gcc_assembler"))]
        {
            // Handle -Wa,--version: print GNU-compatible version string for
            // kernel build system's as-version.sh probe.
            if extra_args.iter().any(|a| a == "--version") {
                println!("GNU assembler (Claude's C Compiler built-in) 2.42");
                return Ok(());
            }

            match self {
                Target::Aarch64 => arm::assembler::assemble(asm_text, output_path),
                Target::X86_64 => x86::assembler::assemble(asm_text, output_path),
                Target::Riscv64 => {
                    riscv::assembler::assemble_with_args(asm_text, output_path, extra_args)
                }
                Target::I686 => i686::assembler::assemble(asm_text, output_path),
            }
        }
    }

    /// Link object files into executable.
    pub(crate) fn link(&self, object_files: &[&str], output_path: &str) -> Result<(), String> {
        self.link_with_args(object_files, output_path, &[])
    }

    /// Link object files with additional user-provided linker args.
    ///
    /// By default, uses the built-in native linker for all architectures.
    /// When the `gcc_linker` Cargo feature is enabled, GCC can be used as a
    /// fallback for operations the built-in linker doesn't support (e.g.,
    /// -shared, -r).
    pub(crate) fn link_with_args(
        &self,
        object_files: &[&str],
        output_path: &str,
        user_args: &[String],
    ) -> Result<(), String> {
        common::link_with_args(&self.linker_config(), object_files, output_path, user_args)
    }
}
