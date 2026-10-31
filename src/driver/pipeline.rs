//! Core compiler driver: struct definition and compilation pipeline.
//!
//! The `Driver` struct holds all configuration parsed from the command line.
//! The compilation pipeline is: preprocess -> lex -> parse -> sema -> lower ->
//! mem2reg -> optimize -> phi-eliminate -> codegen.
//!
//! Submodules handle distinct concerns:
//! - `cli.rs`: GCC-compatible CLI argument parsing
//! - `external_tools.rs`: assembler, linker, and dependency file invocation
//! - `file_types.rs`: input file classification by extension/magic bytes

use crate::backend::Target;
use crate::common::error::{ColorMode, DiagnosticEngine, WarningConfig};
use crate::common::source::SourceManager;
use crate::frontend::lexer::Lexer;
use crate::frontend::parser::Parser;
use crate::frontend::preprocessor::Preprocessor;
use crate::frontend::sema::SemanticAnalyzer;
use crate::ir::lowering::Lowerer;
use crate::ir::mem2reg::{eliminate_phis, promote_allocas};
use crate::passes::run_passes;

/// Compilation mode - determines where in the pipeline to stop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompileMode {
    /// Full compilation: preprocess -> compile -> assemble -> link (default)
    Full,
    /// -S: Stop after generating assembly, output .s file
    AssemblyOnly,
    /// -c: Stop after assembling, output .o file
    ObjectOnly,
    /// -E: Stop after preprocessing, output preprocessed source to stdout
    PreprocessOnly,
    /// -fsyntax-only: run the translation unit through preprocess + parse +
    /// semantic analysis, report diagnostics, and stop. No code is
    /// generated, no output file is produced (`-o` is ignored), and the
    /// link stage never runs — for ANY input kind. This is the driver bug
    /// the i686 header gate worked around with `-E` (WO-4, red-team audit
    /// 2026-09-18): the flag used to be silently dropped by the unknown-
    /// argument handler, so `-x c - -fsyntax-only` linked the (empty) TU
    /// and failed on the missing `main`.
    SyntaxOnly,
}

/// A command-line define: -Dname or -Dname=value
#[derive(Debug, Clone)]
pub(crate) struct CliDefine {
    pub(crate) name: String,
    pub(crate) value: String,
}

/// The compiler driver orchestrates all compilation phases.
///
/// All fields are private; configuration is done through `parse_cli_args()`.
/// Use `has_input_files()` to check if input was provided before calling `run()`.
pub struct Driver {
    pub(super) target: Target,
    pub(super) output_path: String,
    pub(super) output_path_set: bool,
    pub(super) input_files: Vec<String>,
    pub(super) opt_level: u32,
    /// Whether optimization is enabled (any -O level except -O0).
    /// Used to define __OPTIMIZE__ predefined macro.
    pub(super) optimize: bool,
    /// Whether size optimization is requested (-Os or -Oz).
    /// Used to define __OPTIMIZE_SIZE__ predefined macro.
    pub(super) optimize_size: bool,
    /// `-falign-loops[=N[:M]]` / `-fno-align-loops` control of loop-header
    /// padding (Auto = follow the -O level policy).
    pub(super) align_loops: crate::passes::loop_align::AlignControl,
    /// `-falign-jumps[=N[:M]]` / `-fno-align-jumps` control of hot join
    /// point padding under PGO.
    pub(super) align_jumps: crate::passes::loop_align::AlignControl,
    /// `-falign-functions[=N[:M]]` / `-fno-align-functions` control of
    /// function-entry alignment.
    pub(super) align_functions: crate::passes::loop_align::AlignControl,
    /// Permit transformations that reassociate floating-point operations.
    /// False by default: ordinary -O levels preserve source evaluation order.
    /// Enabled explicitly by -ffast-math/-fassociative-math.
    pub(super) fp_reassoc: bool,
    /// Permit fused multiply-add contraction. Kept separate from reassociation:
    /// `-fassociative-math -ffp-contract=off` may vectorize a reduction but must
    /// retain separate multiply and add rounding. Enabled by `-ffast-math` or
    /// an explicit `-ffp-contract=fast`.
    pub(super) fp_contract: crate::common::fp_contract::FpContract,
    /// An explicit -ffp-contract=... was given. GCC's set_fast_math_flags only
    /// touches the contraction mode when it was NOT explicitly set, so
    /// `-ffp-contract=off -ffast-math` keeps contraction OFF while
    /// `-ffast-math -ffp-contract=off` also ends OFF (explicit flag is sticky
    /// in both orders; explicit-vs-explicit remains last-wins).
    pub(super) fp_contract_explicit: bool,
    /// Whether the umbrella -ffast-math option is active.  Kept separate from
    /// fp_reassoc because GCC does not define __FAST_MATH__ for the individual
    /// -fassociative-math/-funsafe-math-optimizations options.
    pub(super) fast_math: bool,
    /// `-fmath-errno`: library math functions report domain errors through
    /// `errno` (C default; cleared by `-fno-math-errno` and `-ffast-math`,
    /// restored by `-fmath-errno` / `-fno-fast-math`, last option wins).
    pub(super) math_errno: bool,
    /// -fwrapv makes signed Add/Sub/Mul modular; never infer no-wrap from C UB.
    pub(super) wrap_signed_overflow: bool,
    /// -ftrapv emits runtime traps on signed integer overflow.
    pub(super) trapv: bool,
    /// -fwhole-program internalizes non-root symbols and merges multi-TU C builds.
    pub(super) whole_program: bool,
    /// -fzero-call-used-regs=<mode> caller-saved register zeroing policy.
    pub(super) zero_call_used_regs: crate::backend::zero_call_used_regs::ZeroCallUsedRegs,
    /// Enabled UBSan checks from -fsanitize= / -fno-sanitize=.
    pub(super) sanitize_enabled: crate::ir::lowering::ubsan::UbsanFlags,
    /// Last enabling `-fsanitize=...` CLI token for untrapped diagnostics.
    pub(super) last_sanitize_arg: Option<String>,
    /// Trap-mode UBSan mask from -fsanitize-trap= / -fsanitize-undefined-trap-on-error.
    pub(super) sanitize_trap: crate::ir::lowering::ubsan::UbsanFlags,
    /// -fno-plt emits GOT-indirect external function calls instead of PLT stubs.
    pub(super) no_plt: bool,
    /// -fvisibility={default,hidden,internal,protected} default visibility for TU definitions.
    pub(super) default_visibility: Option<String>,
    /// -ftrivial-auto-var-init={uninitialized,zero,pattern} policy.
    pub(super) trivial_auto_var_init: crate::ir::lowering::trapv::TrivialAutoVarInit,
    /// `-ffreestanding` / `-fno-hosted`: a freestanding implementation, so
    /// `__STDC_HOSTED__` is 0 (C17 6.10.8.1).  Independent of the builtin
    /// policy: `-ffreestanding -fbuiltin` stays freestanding (GCC 16.2).
    pub(super) freestanding: bool,
    /// The command line's builtin withdrawals (`-fno-builtin[-<fn>]`,
    /// `-ffreestanding`, ...); combined with the TU's own definitions by
    /// `passes::builtin_policy::apply` right after lowering.
    pub(super) builtin_policy: crate::common::pass_policy::BuiltinFlags,
    /// Explicitly observable FP traps; ordinary TUs retain the FENV-off policy.
    pub(super) fp_trapping_math: bool,
    /// Dynamic rounding is observable independently of exception trapping.
    pub(super) fp_rounding_math: bool,
    pub(super) verbose: bool,
    pub(super) mode: CompileMode,
    pub(super) debug_info: bool,
    pub(super) defines: Vec<CliDefine>,
    pub(super) include_paths: Vec<String>,
    /// Quote-only include paths (from -iquote flags).
    /// Searched only for `#include "file"` (quoted includes), after the current
    /// file's directory and before -I paths.
    pub(super) quote_include_paths: Vec<String>,
    /// System include paths (from -isystem flags).
    /// Searched after -I paths, before default system paths.
    pub(super) isystem_include_paths: Vec<String>,
    /// After include paths (from -idirafter flags).
    /// Searched last, after all other include paths.
    pub(super) after_include_paths: Vec<String>,
    /// Library search paths (from -L flags)
    pub(super) linker_paths: Vec<String>,
    /// Ordered list of linker items preserving command-line order.
    /// Contains passthrough object/archive paths, -l flags, and -Wl, flags
    /// in the order they appeared on the command line. This is critical for
    /// flags like -Wl,--whole-archive which must precede the .a they affect.
    pub(super) linker_ordered_items: Vec<String>,
    /// Whether to link statically (-static)
    pub(super) static_link: bool,
    /// Whether to produce a shared library (-shared)
    pub(super) shared_lib: bool,
    /// Whether to omit standard library linking (-nostdlib)
    pub(super) nostdlib: bool,
    /// `-nostartfiles`: no crt1/crti/crtbegin/crtend/crtn, libraries kept
    pub(super) nostartfiles: bool,
    /// `-nodefaultlibs`: startup objects kept, every default library dropped
    pub(super) nodefaultlibs: bool,
    /// Whether to produce a relocatable object file (-r / -relocatable)
    pub(super) relocatable: bool,
    /// Whether to generate fully interposable position-independent code
    /// (`-fPIC`/`-fpic`, or `-shared`).
    pub(super) pic: bool,
    /// Whether to generate a position-independent executable (`-fPIE`/
    /// `-fpie`, the x86-64 Linux default). PIE permits direct references to
    /// ordinary executable data where full PIC requires the GOT.
    pub(super) pie: bool,
    /// `-ftls-model=` floor (GCC `flag_tls_default`); see common::tls.
    pub(super) tls_model_floor: Option<crate::common::tls::TlsModel>,
    /// Last `-mtls-dialect=` value, validated against the final target.
    pub(super) tls_dialect: Option<String>,
    /// Files to force-include before the main source (-include flag)
    pub(super) force_includes: Vec<String>,
    /// Whether to replace `ret` with `jmp __x86_return_thunk` (-mfunction-return=thunk-extern)
    pub(super) function_return_thunk: bool,
    /// Whether to replace indirect calls/jumps with retpoline thunks (-mindirect-branch=thunk-extern)
    pub(super) indirect_branch_thunk: bool,
    /// Patchable function entry: (total_nops, nops_before_entry).
    /// Set by -fpatchable-function-entry=N[,M] where N is total NOPs and M is
    /// how many go before the entry point (the rest go after).
    /// Used by the Linux kernel for ftrace and static call patching.
    pub(super) patchable_function_entry: Option<(u32, u32)>,
    /// Function-entry mcount sub-mode configuration, mutated by -mfentry /
    /// -mrecord-mcount / -mnop-mcount. Inert on its own; only takes effect
    /// when mcount_pg is true (matching GCC: -pg is the trigger, the -m flags
    /// only configure how the call is emitted).
    pub(super) mcount_submode: crate::backend::McountInstrumentation,
    /// Set by -pg: activates function-entry mcount instrumentation using the
    /// configured mcount_submode. The kernel's CFLAGS_REMOVE mechanism strips
    /// -pg (e.g. for VDSO objects) leaving the sub-modes inert, matching GCC.
    pub(super) mcount_pg: bool,
    /// S13: set by -finstrument-functions. Runs the IR-level
    /// instrumentation pass (__cyg_profile_func_enter/exit calls) over every
    /// defined function that does not carry no_instrument_function.
    pub(super) instrument_functions: bool,
    /// Whether to emit endbr64 at function entry points (-fcf-protection=branch).
    /// Required by the Linux kernel for Intel CET/IBT (Indirect Branch Tracking).
    pub(super) cf_protection_branch: bool,
    /// GCC __CET__ macro value derived from -fcf-protection
    /// (None = flag absent or =none; "1" branch, "2" return, "3" full).
    pub(super) cf_protection_value: Option<&'static str>,
    /// `-fstack-clash-protection`: probe 4096-byte pages of static/dynamic frames.
    pub(super) stack_clash_protection: bool,
    /// `-fstack-protector{,-strong,-all,-explicit}` mode.
    pub(super) stack_protector_mode: crate::backend::stack_protector::StackProtectorMode,
    /// `--param=ssp-buffer-size=N` threshold for `-fstack-protector` (default 8).
    pub(super) ssp_buffer_size: usize,
    /// `-mstack-protector-guard*` configuration.
    pub(super) stack_protector_guard: crate::backend::stack_protector::StackProtectorGuard,
    /// Whether SSE is disabled (-mno-sse). When true, the compiler must not emit
    /// any SSE/SSE2/AVX instructions (movdqu, movss, movsd, etc.).
    /// The Linux kernel uses -mno-sse to avoid FPU state in kernel code.
    pub(super) no_sse: bool,
    /// SSE2 is denied directly or as part of `-mno-sse`. Unlike `no_sse`,
    /// this can coexist with SSE-only i686 code after explicit `-msse`.
    pub(super) no_sse2: bool,
    /// A direct `-mno-sse2` / general-register denial. This survives a later
    /// SSE-only grant, whereas `-mno-sse -msse` restores the baseline SSE2
    /// state when no direct SSE2 denial was present (GCC 16.2 flag matrix).
    pub(super) sse2_explicitly_disabled: bool,
    /// Set by an explicit `-mno-sse`. `-march=native` and CPU profiles must
    /// not re-enable SSE after that: GCC keeps the disable (kernel
    /// decompressor: `-mno-sse` then Cachy `-march=native`).
    pub(super) sse_explicitly_disabled: bool,
    /// Set by `-mno-avx`/`-mno-sse4.x`/`-mno-sse`: the TU explicitly forbade
    /// the VEX encoding itself (every VEX form, 128-bit included).
    /// Distinct from `enable_avx` (which records what was explicitly
    /// *requested* and drives `__AVX__`): LCCC's x86-64 code-generation
    /// baseline is x86-64 (no VEX), so AVX is granted only by an explicit
    /// flag or `-march=` profile; this denial also beats such a grant.
    /// `-mno-avx2` does NOT set this — AVX2 is the 256-bit integer class,
    /// a strict subset of what the VEX encoding buys: VEX.128 forms
    /// (`vmovsd`, `vaddsd`, `vfmadd231sd`, …) require only AVX, and
    /// `-march=x86-64-v3 -mno-avx2` (the AVX1+FMA target class) must keep
    /// them, exactly like GCC. That denial is [`Self::avx2_explicitly_disabled`].
    pub(super) avx_explicitly_disabled: bool,
    /// Set by `-mno-avx2`: the 256-bit class (AVX2 integer ops and every
    /// `ymm` use) is forbidden while the VEX.128 world stays legal. The
    /// AVX2 class is off unless `-mavx2` or a `-march=` profile grants it;
    /// this denial beats such a grant, and the VEX.128 world stays legal.
    pub(super) avx2_explicitly_disabled: bool,
    /// Set by `-mno-fma` (and by any AVX/SSE denial, since `vfmadd*` is
    /// VEX-encoded and requires AVX): `vfmadd*` must not be emitted.
    pub(super) fma_explicitly_disabled: bool,
    /// Set by `-mno-sse4.1` / `-mno-sse4` / `-mno-ssse3` / `-mno-sse3`
    /// (each of which denies SSE4.1 as well): no SSE4.1 encodings (`pmulld`,
    /// `roundsd`, `blendvpd`, `pmovsx*`, …).  Separate from
    /// `avx_explicitly_disabled` because `-mno-avx` alone keeps SSE4.1 legal
    /// (x86-64-v2 hardware).
    pub(super) sse41_explicitly_disabled: bool,
    /// Set by `-mno-bmi`: scalar ANDN / BEXTR-family encodings forbidden.
    /// Kept distinct from `enable_bmi` so the denial stays sticky against a
    /// later `-march=` profile that grants BMI1.
    pub(super) bmi_explicitly_disabled: bool,
    /// Set by `-mno-bmi2`: SHLX/SHRX/SARX/RORX/MULX/PEXT/PDEP forbidden.
    pub(super) bmi2_explicitly_disabled: bool,
    /// Set by `-mno-lzcnt`: LZCNT/TZCNT forbidden (the F3 0F BD/BC
    /// encodings); the Clz/Ctz lowering falls back to BSR/BSF + fixup.
    pub(super) lzcnt_explicitly_disabled: bool,
    /// Set by `-mno-popcnt`: POPCNT (0F B8) forbidden; the bit-count
    /// lowering falls back to the shr/adc loop.
    pub(super) popcnt_explicitly_disabled: bool,
    /// `-mprfchw` or a PRFCHW `-march=` CPU: `prefetchw` is available.
    /// Arch-implied like every `enable_*` bit; the explicit flags are
    /// replayed.
    pub(super) enable_prfchw: bool,
    /// Sticky `-mno-prfchw` (GCC: wins over a later PRFCHW `-march=`).
    pub(super) prfchw_explicitly_disabled: bool,
    /// GCC's ABM (`-mabm` or an ABM `-march=` CPU, `__ABM__`): POPCNT, and
    /// LZCNT unless an LZCNT flag decided it ([`Driver::resolved_lzcnt`]).
    pub(super) enable_abm: bool,
    /// `-mno-abm` was the last ABM decision: a `-march=` CPU then withholds
    /// what it has only through ABM
    /// ([`crate::backend::x86::arch_table::X86ArchRow::abm_derived`]).
    pub(super) abm_explicitly_disabled: bool,
    /// Sticky `-mno-mmx`: no `__MMX__` (GCC 16.2; the MMX intrinsics stay,
    /// lccc emulates them without the MMX registers).
    pub(super) mmx_explicitly_disabled: bool,

    /// Explicit `-msse` (and, as its weaker spelling, `-msse2`). On x86-64
    /// SSE is baseline and only `no_sse` matters; on i686 the default
    /// baseline is the x86-64 ISA level (SSE/SSE2 on — GCC 16.2's default
    /// `-m32` resolves to `-march=x86-64`, measured), so this positive grant
    /// only matters after an explicit pre-SSE `-march=` or `-mno-sse2`
    /// denial. Every higher SSE/AVX grant is visible through its own
    /// `enable_*` bit.
    pub(super) enable_sse: bool,
    /// Explicit `-msse2` only (the SSE2 level, above `-msse`). i686 resolves
    /// its SSE2 baseline through this bit, the SSE-family `enable_*` bits
    /// (SSE3 and up all imply SSE2), and the default-baseline grant — see
    /// [`Self::resolved_i686_sse2`]. x86-64 codegen treats SSE2 as an
    /// inescapable baseline, so there this bit feeds nothing: a `-mno-sse2`
    /// denial is normalized to the full `-mno-sse` state at the end of parse
    /// instead.
    pub(super) enable_sse2: bool,
    /// The final accepted `-march=` for an i686 invocation names a pre-SSE
    /// micro-architecture (`i386`/`i486`/`i586`/`i686`/`pentium`/
    /// `pentiumpro`/`pentium-mmx`), which vetoes the default x86-64-baseline
    /// SSE/SSE2/MMX grant (GCC 16.2 `-m32 -march=i686` measures
    /// `-msse`/`-msse2` [disabled] and no `__MMX__`). Set by the `-march=`
    /// handler per accepted spelling (last `-march=` wins, like the arch
    /// profile it selects); explicit `-msse`/`-msse2` still override the
    /// veto at resolve time.
    pub(super) march_pre_sse: bool,
    /// Whether the final `-march=` CPU carries FXSR (`F::Fxsr` in the
    /// generated ISA table). Drives `__FXSR__` on i686; x86-64 defines it
    /// unconditionally (measured on GCC 16.2).
    pub(super) march_fxsr: bool,
    /// Explicit `-mfxsr`/`-mno-fxsr` (last one wins; measured: GCC's flag
    /// overrides even the `-march` row, e.g. `-march=k6 -mfxsr` defines
    /// `__FXSR__` and `-march=pentium4 -mno-fxsr` drops it).
    pub(super) fxsr_override: Option<bool>,
    /// The pre-SSE2 `-march=` CPU has MMX (Pentium MMX/II/III, K6, WinChip,
    /// the Athlons): GCC 16.2 keeps `__MMX__` there, `-mno-sse` included.
    /// Meaningful only with `march_pre_sse`; set with it per `-march=`.
    pub(super) march_pre_sse_mmx: bool,
    /// Set by `-mno-movbe`: MOVBE forbidden.
    pub(super) movbe_explicitly_disabled: bool,
    /// Set by `-mno-cx16`: CMPXCHG16B forbidden.
    pub(super) cx16_explicitly_disabled: bool,
    /// Explicit x86 ISA `-m`/`-mno-` flags in command-line order, for the
    /// `-march=` replace-and-replay contract (GCC-exact, measured on the
    /// host oracle, gcc 14.2 `-dM -E`):
    ///
    /// * a later `-march=` REPLACES the arch-implied set (it is not a
    ///   union — `-march=x86-64-v3 -march=x86-64` leaves `__BMI__`/
    ///   `__MOVBE__`/`__AVX__` all absent);
    /// * an explicit `-m*`/`-mno-*` decision overrides the FINAL arch in
    ///   BOTH directions and in EITHER order (`-mno-bmi -march=v3` →
    ///   `__BMI__` absent; `-mbmi -march=x86-64` → `__BMI__` defined);
    /// * among explicit flags, the later one wins.
    ///
    /// Each `-march=` therefore resets the enable surface to the baseline
    /// and replays these recorded flags AFTER applying the profile, so the
    /// explicit decisions land on top of the final arch set exactly once
    /// more.  The sticky `*_explicitly_disabled` denials and `no_sse` are
    /// consulted at resolve time and survive the reset untouched.
    pub(super) x86_explicit_isa_args: Vec<String>,
    pub(super) skip_rax_setup: bool,
    /// GCC's MASK_80387 cleared (`-mno-80387`, `-msoft-float`,
    /// `-mgeneral-regs-only`; last-wins against `-m80387`/`-mhard-float`):
    /// x87 instructions are fenced out of the final text.
    pub(super) no_x87: bool,
    /// `-mno-fp-ret-in-387`: x87 arithmetic stays legal, but FP values are
    /// not returned in `%st(0)`. lccc does not implement the GPR return
    /// convention, so every function boundary that would need it is refused.
    pub(super) no_fp_ret_in_387: bool,
    /// -mindirect-branch=thunk-inline: expand the full retpoline at each
    /// indirect branch site (kernel vDSO: userspace code, no kernel thunks).
    pub(super) indirect_branch_thunk_inline: bool,
    /// -mindirect-branch-cs-prefix: CS prefix before thunk calls.
    #[allow(dead_code)]
    pub(super) indirect_branch_cs_prefix: bool,
    /// Explicit SIMD feature flags from -msse3, -msse4.1, -mavx, -mavx2, etc.
    /// When set, the corresponding __SSE3__, __AVX__, __AVX2__ macros are defined.
    /// These flags enable projects like blosc to compile SIMD-optimized code paths.
    pub(super) enable_sse3: bool,
    pub(super) enable_ssse3: bool,
    pub(super) enable_sse4_1: bool,
    pub(super) enable_sse4_2: bool,
    pub(super) enable_avx: bool,
    pub(super) enable_avx2: bool,
    pub(super) enable_aes: bool,
    pub(super) enable_pclmul: bool,
    pub(super) enable_f16c: bool,
    pub(super) enable_fma: bool,
    pub(super) enable_bmi: bool,
    pub(super) enable_bmi2: bool,
    pub(super) enable_lzcnt: bool,
    pub(super) enable_popcnt: bool,
    pub(super) enable_movbe: bool,
    /// `-mcx16` (or a v2+ profile): CMPXCHG16B granted.
    pub(super) enable_cx16: bool,
    pub(super) enable_rdrnd: bool,
    // AVX-512 family (accepted for -march/-m flag completeness; the backend
    // lowers AVX-512 only where intrinsic coverage exists, and runtime
    // dispatch must verify host support before use).
    pub(super) enable_avx512f: bool,
    pub(super) enable_avx512cd: bool,
    pub(super) enable_avx512dq: bool,
    pub(super) enable_avx512bw: bool,
    pub(super) enable_avx512vl: bool,
    pub(super) enable_avx512ifma: bool,
    pub(super) enable_avx512vbmi: bool,
    pub(super) enable_avx512vbmi2: bool,
    pub(super) enable_avx512vnni: bool,
    pub(super) enable_avx512bitalg: bool,
    pub(super) enable_avx512vpopcntdq: bool,
    pub(super) enable_avx512bf16: bool,
    pub(super) enable_avx512fp16: bool,
    pub(super) enable_avx512er: bool,
    pub(super) enable_avx512pf: bool,
    pub(super) enable_avx512vp2intersect: bool,
    pub(super) enable_avx512bmm: bool,
    // AVX-VNNI / AVX10 / AVX-IFMA / AVX-NE-CONVERT / GFNI / VAES / VPCLMULQDQ
    pub(super) enable_avxvnni: bool,
    pub(super) enable_avxifma: bool,
    pub(super) enable_avxneconvert: bool,
    pub(super) enable_avx10_1: bool,
    pub(super) enable_avx10_2: bool,
    pub(super) enable_gfni: bool,
    pub(super) enable_vaes: bool,
    pub(super) enable_vpclmulqdq: bool,
    pub(super) enable_avxvnniint8: bool,
    pub(super) enable_avxvnniint16: bool,
    pub(super) enable_sha512: bool,
    pub(super) enable_sm3: bool,
    pub(super) enable_sm4: bool,
    pub(super) enable_movrs: bool,
    pub(super) enable_user_msr: bool,
    pub(super) enable_apxf: bool,
    pub(super) enable_amx_tile: bool,
    pub(super) enable_amx_int8: bool,
    pub(super) enable_amx_bf16: bool,
    pub(super) enable_cmpccxadd: bool,
    /// Accepted x86 scheduling tune name from -mtune=. LCCC does not yet have
    /// a machine scheduler, so this is deliberately diagnostic-only; ISA
    /// selection belongs to -march= and is never silently conflated with tune.
    pub(super) x86_tune: Option<String>,
    /// The `-march=` spelling as given (x86 only); the tuning row falls back
    /// to it when `-mtune` is absent, exactly like GCC.
    pub(super) x86_march: Option<String>,
    /// Whether to use only general-purpose registers (-mgeneral-regs-only).
    /// On AArch64, this prevents FP/SIMD register usage. The Linux kernel uses
    /// this to avoid touching NEON/FP state. When set, variadic function prologues
    /// must not save q0-q7, and va_start sets __vr_offs=0 (no FP save area).
    pub(super) general_regs_only: bool,
    /// Whether to use the kernel code model (-mcmodel=kernel). All symbols
    /// are assumed to be in the negative 2GB of the virtual address space.
    /// Uses absolute sign-extended 32-bit addressing (movq $symbol) for
    /// global address references, producing R_X86_64_32S relocations.
    pub(super) code_model_kernel: bool,
    /// Whether to disable jump table emission for switch statements (-fno-jump-tables).
    /// The Linux kernel uses this with -mindirect-branch=thunk-extern (retpoline) to
    /// prevent indirect jumps that objtool would reject.
    pub(super) no_jump_tables: bool,
    /// RISC-V ABI override from -mabi= flag (e.g., "lp64", "lp64d", "lp64f").
    /// When set, overrides the default "lp64d" passed to the assembler.
    /// The Linux kernel uses -mabi=lp64 (soft-float) for kernel code.
    pub(super) riscv_abi: Option<String>,
    /// RISC-V architecture override from -march= flag (e.g., "rv64imac_zicsr_zifencei").
    /// When set, overrides the default "rv64gc" passed to the assembler.
    pub(super) riscv_march: Option<String>,
    /// RISC-V -mno-relax flag: suppress linker relaxation.
    /// When true, the codegen emits `.option norelax` and the assembler is
    /// invoked with `-mno-relax`. This prevents R_RISCV_RELAX relocations
    /// that would allow the linker to introduce absolute symbol references.
    /// Required by the Linux kernel's EFI stub (built with -fpic -mno-relax).
    pub(super) riscv_no_relax: bool,
    /// Explicit language override from -x flag.
    /// When set, overrides file extension detection for input language.
    /// Values: "c", "assembler", "assembler-with-cpp", "none" (reset).
    /// Used for stdin input ("-") and also for files like /dev/null where
    /// the extension doesn't indicate the language.
    pub(super) explicit_language: Option<String>,
    /// Extra arguments to pass to the assembler (from -Wa, flags).
    /// Used by kernel build to query assembler version via -Wa,--version.
    pub(super) assembler_extra_args: Vec<String>,
    /// Path to write dependency file (from -MF or -Wp,-MMD,path or -Wp,-MD,path).
    /// When set, the compiler writes a Make-compatible dependency file listing
    /// the input source as a dependency of the output object. Used by the Linux
    /// kernel build system (fixdep) to track header dependencies.
    pub(super) dep_file: Option<String>,
    /// Whether to prepend `.code16gcc` to the assembly output (-m16).
    /// When true, generated 32-bit i686 code is prefixed with `.code16gcc` so
    /// the assembler adds operand/address-size override prefixes for 16-bit
    /// real mode execution. Used by the Linux kernel boot code.
    pub(super) code16gcc: bool,
    /// Whether -M or -MM was specified (dependency-only mode).
    /// In this mode, the compiler preprocesses the source and outputs
    /// make-compatible dependency rules instead of compiling. -M includes
    /// system headers in the output, -MM skips them (but our minimal
    /// implementation doesn't list headers at all).
    pub(super) dep_only: bool,
    /// Dependency rule target from -MT flag (e.g., `-MT src/foo.o`).
    /// When set, overrides the default target in the dependency rule.
    /// Default: derive from input filename (replace extension with .o).
    pub(super) dep_target: Option<String>,
    /// -MMD/-MM system-directory filter: when true, headers resolved in a
    /// system directory (-isystem, default system dirs, -idirafter, the
    /// bundled-header set) are omitted from make dependencies.  GCC's
    /// filter is directory-based, so the verdict comes from the include
    /// search step that matched (preprocessor dep tracking), NOT from the
    /// `<>` vs `""` spelling of the directive.
    pub(super) dep_exclude_system: bool,
    /// -MP: after the main rule, emit one phony rule per prerequisite so
    /// deleting a prerequisite doesn't break an unchanged make graph.
    pub(super) dep_phony: bool,
    /// Make-dependency files actually opened for the CURRENT input, in
    /// first-open order, with their system-directory verdicts.  Published
    /// by the preprocessor run inside the pipeline (compile / -E) and
    /// consumed by write_dep_file.  RefCell because the pipeline takes
    /// &self per compile while the preprocessor is method-local.
    pub(super) last_dep_files: std::cell::RefCell<Vec<(std::path::PathBuf, bool)>>,
    /// Whether to suppress line markers in preprocessor output (-P flag).
    /// When true, `# <line> "<file>"` directives are stripped from -E output.
    /// Used by the Linux kernel's cc-version.sh to detect the compiler.
    pub(super) suppress_line_markers: bool,
    /// Whether -nostdinc was passed. When delegating to gcc for assembly
    /// preprocessing, this must be forwarded to prevent system header
    /// interference.
    pub(super) nostdinc: bool,
    /// Macro undefinitions from -U flags. These need to be forwarded when
    /// delegating preprocessing to gcc (e.g., -Uriscv for kernel linker scripts).
    pub(super) undef_macros: Vec<String>,
    /// Whether -undef was passed. Prevents the preprocessor from predefining
    /// any system-specific or GCC-specific macros. Must be forwarded when
    /// delegating to gcc (e.g., for DTS preprocessing in kernel builds).
    pub(super) undef_all: bool,
    /// Warning configuration parsed from -W flags. Controls which warnings are
    /// enabled, disabled, or promoted to errors. Processed left-to-right from
    /// the command line to match GCC semantics.
    pub(super) warning_config: WarningConfig,
    /// Whether GNU C extensions are enabled. Defaults to true.
    /// Set to false when -std=c99, -std=c11, etc. (strict ISO C mode) is used.
    /// When false, bare GNU keywords like `typeof` and `asm` are treated as
    /// identifiers (the __typeof__/__asm__ forms always work).
    pub(super) gnu_extensions: bool,
    /// Whether to place each function in its own section (-ffunction-sections).
    pub(super) function_sections: bool,
    /// Whether to place each data object in its own section (-fdata-sections).
    pub(super) data_sections: bool,
    /// Whether GNU89 inline semantics are in effect (-fgnu89-inline or -std=gnu89).
    /// When true, the preprocessor defines __GNUC_GNU_INLINE__ instead of __GNUC_STDC_INLINE__.
    /// This affects how projects like mpack select their inline linkage model.
    pub(super) gnu89_inline: bool,
    /// Set when -fgnu89-inline / -fno-gnu89-inline was given EXPLICITLY on the
    /// command line. GCC treats an explicit -fgnu89-inline/-fno-gnu89-inline as
    /// overriding any later -std= (which only implies the default inline model).
    /// Without this, glibc's `-std=gnu11 -fgnu89-inline` plus a trailing
    /// `-std=gnu17` from a driver wrapper would silently revert to C99 inline
    /// semantics, turning `extern __always_inline` into an external definition
    /// and causing "multiple definition" errors at partial-link time.
    pub(super) gnu89_inline_explicit: Option<bool>,
    /// `__STDC_VERSION__` selected by the active C dialect.
    /// The default is GNU C23 (`202311L`); an explicit `-std=` replaces it.
    /// `Some(None)` means the dialect predates the macro (C89/C90), while
    /// `Some(Some(v))` defines it as `v`.
    pub(super) stdc_version: Option<Option<&'static str>>,
    /// `-fno-tree-vectorize` disables automatic vectorizer passes while
    /// retaining explicit vector types and intrinsics.
    pub(super) disable_tree_vectorize: bool,
    /// Opt in to the module-wide Andersen-style points-to and mod/ref pass.
    /// Like GCC, this is deliberately independent of the -O level.
    pub(super) ipa_pta: bool,
    /// Opt in to the pressure-guided, dependence-preserving IR scheduler.
    pub(super) live_range_shrinkage: bool,
    /// Whether -fexceptions is in effect (defines __EXCEPTIONS like GCC).
    pub(super) exceptions: bool,
    /// Whether to dump preprocessor defines instead of preprocessed output (-dM).
    /// When true with -E, outputs `#define NAME VALUE` for all predefined and
    /// user-defined macros instead of the preprocessed source. Used by build
    /// systems like Meson to detect compiler version via __GNUC__ etc.
    pub(super) dump_defines: bool,
    /// Color mode for diagnostic output, controlled by -fdiagnostics-color={auto,always,never}.
    /// Defaults to Auto (colorize when stderr is a terminal).
    pub(super) color_mode: ColorMode,
    /// Whether to use COMMON linkage for tentative definitions (-fcommon).
    /// When true, uninitialized global variables (tentative definitions) are emitted
    /// as COMMON symbols, allowing multiple TUs to define the same global without
    /// linker "multiple definition" errors. GCC defaulted to -fcommon before GCC 10.
    /// When false (-fno-common, the default), tentative definitions go to BSS with
    /// strong linkage, and duplicate definitions cause link errors.
    pub(super) fcommon: bool,
    /// Number of integer arguments to pass in registers for i686 (-mregparm=N).
    /// 0 = standard cdecl (all on stack), 1-3 = pass first N int args in EAX/EDX/ECX.
    pub(super) regparm: u8,
    /// Requested stack boundary in bytes (-mpreferred-stack-boundary /
    /// -mstack-alignment). 16 = SysV default; the kernel realmode code
    /// passes 4 (boundary=2). Only i686 honours values below 16.
    pub(super) preferred_stack_bytes: u8,
    /// Whether to omit the frame pointer (-fomit-frame-pointer).
    pub(super) omit_frame_pointer: bool,
    /// Whether to suppress .eh_frame unwind table generation
    /// (-fno-asynchronous-unwind-tables / -fno-unwind-tables).
    pub(super) no_unwind_tables: bool,
    /// Raw CLI arguments (excluding argv[0], -o, output path, and input files).
    /// Used for GCC -m16 passthrough: we forward all flags directly to GCC
    /// rather than trying to reconstruct them from parsed state.
    pub(super) raw_args: Vec<String>,
    /// Whether -pthread was specified. When true, the preprocessor defines
    /// _REENTRANT=1 (matching GCC/Clang behavior). Build systems that detect
    /// pthread support via configure (ax_pthread.m4) add -lpthread themselves.
    pub(super) pthread: bool,
    /// PGO: profile generation directory/file (from -fprofile-generate[=path])
    pub(super) pgo_generate: Option<String>,
    /// PGO: profile use path (from -fprofile-use[=path], -fprofile-use, -fbranch-probabilities, -fauto-profile)
    pub(super) pgo_use: Option<String>,
    /// PGO: counter update mode ("single" default, or "atomic"
    /// from -fprofile-update=atomic).
    pub(super) pgo_update: Option<String>,
}

impl Driver {
    pub fn new() -> Self {
        Self {
            target: Target::X86_64,
            output_path: "a.out".to_string(),
            output_path_set: false,
            input_files: Vec::new(),
            opt_level: 2,    // All levels run the same optimizations; default to max
            optimize: false, // Only set to true when user explicitly passes -O1 or higher
            optimize_size: false,
            align_loops: crate::passes::loop_align::AlignControl::Auto,
            align_jumps: crate::passes::loop_align::AlignControl::Auto,
            align_functions: crate::passes::loop_align::AlignControl::Auto,
            fp_reassoc: false,
            math_errno: true,
            // Contraction default for C: FAST, matching GCC's C default
            // (-ffp-contract=fast). The previous Off default claimed to match
            // "GCC 14 behavior in loops" -- refuted by the godbolt oracles:
            // on a cross-statement reduction at -O3 -march=x86-64-v3, gcc16.2
            // and icx fuse to vfmadd by default (clang, whose default is
            // `on`, does not). Off forfeited every scalar FMA against the
            // reference compilers (dot8: lccc 46 insns vs gcc 21 / icx 10).
            // Explicit -ffp-contract={on,off} wins; the backend additionally
            // gates on FMA3 ISA availability, so baseline x86-64 builds are
            // numerically unchanged.
            fp_contract: crate::common::fp_contract::FpContract::c_language_default(),
            fp_contract_explicit: false,
            fast_math: false,
            wrap_signed_overflow: false,
            trapv: false,
            whole_program: false,
            zero_call_used_regs: crate::backend::zero_call_used_regs::ZeroCallUsedRegs::Skip,
            sanitize_enabled: crate::ir::lowering::ubsan::UbsanFlags::NONE,
            last_sanitize_arg: None,
            sanitize_trap: crate::ir::lowering::ubsan::UbsanFlags::NONE,
            no_plt: false,
            default_visibility: None,
            trivial_auto_var_init: crate::ir::lowering::trapv::TrivialAutoVarInit::Uninitialized,
            freestanding: false,
            builtin_policy: crate::common::pass_policy::BuiltinFlags::default(),
            fp_trapping_math: false,
            fp_rounding_math: false,
            verbose: false,
            mode: CompileMode::Full,
            debug_info: false,
            defines: Vec::new(),
            include_paths: Vec::new(),
            quote_include_paths: Vec::new(),
            isystem_include_paths: Vec::new(),
            after_include_paths: Vec::new(),
            linker_paths: Vec::new(),
            linker_ordered_items: Vec::new(),
            static_link: false,
            shared_lib: false,
            nostdlib: false,
            nostartfiles: false,
            nodefaultlibs: false,
            relocatable: false,
            // Debian x86-64 defaults to PIE, not full PIC. Keeping these modes
            // distinct lets executable data use direct RIP-relative accesses
            // while explicit -fPIC/-shared remains interposition-safe.
            pic: false,
            pie: true,
            tls_model_floor: None,
            tls_dialect: None,
            force_includes: Vec::new(),
            function_return_thunk: false,
            indirect_branch_thunk: false,
            patchable_function_entry: None,
            mcount_submode: crate::backend::McountInstrumentation::default(),
            mcount_pg: false,
            instrument_functions: false,
            cf_protection_branch: false,
            cf_protection_value: None,
            stack_clash_protection: false,
            stack_protector_mode: crate::backend::stack_protector::StackProtectorMode::None,
            ssp_buffer_size: 8,
            stack_protector_guard: crate::backend::stack_protector::StackProtectorGuard::default(),
            no_sse: false,
            no_sse2: false,
            sse2_explicitly_disabled: false,
            sse_explicitly_disabled: false,
            avx_explicitly_disabled: false,
            avx2_explicitly_disabled: false,
            fma_explicitly_disabled: false,
            sse41_explicitly_disabled: false,
            bmi_explicitly_disabled: false,
            bmi2_explicitly_disabled: false,
            lzcnt_explicitly_disabled: false,
            popcnt_explicitly_disabled: false,
            enable_prfchw: false,
            prfchw_explicitly_disabled: false,
            enable_abm: false,
            abm_explicitly_disabled: false,
            mmx_explicitly_disabled: false,
            enable_sse: false,
            enable_sse2: false,
            march_pre_sse: false,
            march_pre_sse_mmx: false,
            march_fxsr: true,
            fxsr_override: None,
            movbe_explicitly_disabled: false,
            cx16_explicitly_disabled: false,
            x86_explicit_isa_args: Vec::new(),
            skip_rax_setup: false,
            no_x87: false,
            no_fp_ret_in_387: false,
            indirect_branch_thunk_inline: false,
            indirect_branch_cs_prefix: false,
            enable_sse3: false,
            enable_ssse3: false,
            enable_sse4_1: false,
            enable_sse4_2: false,
            enable_avx: false,
            enable_avx2: false,
            enable_aes: false,
            enable_pclmul: false,
            enable_f16c: false,
            enable_fma: false,
            enable_bmi: false,
            enable_bmi2: false,
            enable_lzcnt: false,
            enable_popcnt: false,
            enable_movbe: false,
            enable_cx16: false,
            enable_rdrnd: false,
            enable_avx512f: false,
            enable_avx512cd: false,
            enable_avx512dq: false,
            enable_avx512bw: false,
            enable_avx512vl: false,
            enable_avx512ifma: false,
            enable_avx512vbmi: false,
            enable_avx512vbmi2: false,
            enable_avx512vnni: false,
            enable_avx512bitalg: false,
            enable_avx512vpopcntdq: false,
            enable_avx512bf16: false,
            enable_avx512fp16: false,
            enable_avx512er: false,
            enable_avx512pf: false,
            enable_avx512vp2intersect: false,
            enable_avx512bmm: false,
            enable_avxvnni: false,
            enable_avxifma: false,
            enable_avxneconvert: false,
            enable_avx10_1: false,
            enable_avx10_2: false,
            enable_gfni: false,
            enable_vaes: false,
            enable_vpclmulqdq: false,
            enable_avxvnniint8: false,
            enable_avxvnniint16: false,
            enable_sha512: false,
            enable_sm3: false,
            enable_sm4: false,
            enable_movrs: false,
            enable_user_msr: false,
            enable_apxf: false,
            enable_amx_tile: false,
            enable_amx_int8: false,
            enable_amx_bf16: false,
            enable_cmpccxadd: false,
            x86_tune: None,
            x86_march: None,
            general_regs_only: false,
            code_model_kernel: false,
            no_jump_tables: false,
            riscv_abi: None,
            riscv_march: None,
            riscv_no_relax: false,
            explicit_language: None,
            assembler_extra_args: Vec::new(),
            dep_file: None,
            code16gcc: false,
            dep_only: false,
            dep_target: None,
            dep_exclude_system: false,
            dep_phony: false,
            last_dep_files: std::cell::RefCell::new(Vec::new()),
            suppress_line_markers: false,
            nostdinc: false,
            undef_macros: Vec::new(),
            undef_all: false,
            warning_config: WarningConfig::new(),
            gnu_extensions: true,
            function_sections: false,
            data_sections: false,
            gnu89_inline: false,
            gnu89_inline_explicit: None,
            stdc_version: Some(Some("202311L")), // GCC 16.2 default: -std=gnu23
            disable_tree_vectorize: false,
            ipa_pta: false,
            live_range_shrinkage: false,
            exceptions: false,
            dump_defines: false,
            color_mode: ColorMode::Auto,
            fcommon: false,
            regparm: 0,
            preferred_stack_bytes: 16,
            omit_frame_pointer: false,
            no_unwind_tables: false,
            raw_args: Vec::new(),
            pthread: false,
            pgo_generate: None,
            pgo_use: None,
            pgo_update: None,
        }
    }

    /// Whether the driver has any input files to process.
    pub fn has_input_files(&self) -> bool {
        !self.input_files.is_empty()
    }

    /// True when `-o -` asks for standard output.
    ///
    /// GCC treats `-o -` as "write to stdout" for the modes that stream text
    /// (`-S`, `-E`, and the `-M*` dependency printers) and rejects it for `-c`
    /// (its assembler fails with "can't open a bfd on stdout") — either way it
    /// never creates a file called `-`. lccc used to create that file for
    /// every mode, which silently swallowed the output of
    ///   `lccc -S -o - t.c | ...`   and `lccc -E -o - t.c | ...`.
    pub(crate) fn output_is_stdout(&self) -> bool {
        self.output_path_set && self.output_path == "-"
    }

    /// Write textual output (assembly, preprocessed source, dependency rules)
    /// to `-o`'s destination, honouring `-o -` == stdout.
    ///
    /// Every text producer in the driver funnels through here so the `-` rule
    /// exists exactly once; binary producers (objects, executables) must not
    /// come near it, because a streamed ELF is not a thing any consumer can
    /// use and GCC errors out instead.
    fn write_text_output(&self, text: &str) -> Result<(), String> {
        if self.output_is_stdout() {
            use std::io::Write;
            let stdout = std::io::stdout();
            let mut lock = stdout.lock();
            lock.write_all(text.as_bytes())
                .and_then(|()| lock.flush())
                .map_err(|e| format!("Cannot write to standard output: {}", e))
        } else {
            std::fs::write(&self.output_path, text)
                .map_err(|e| format!("Cannot write {}: {}", self.output_path, e))
        }
    }

    /// Reject `-o -` where GCC rejects it, before any work is done.
    ///
    /// * `-c`: the object is a binary; `as` cannot emit one on stdout, so GCC
    ///   aborts — we say so once, up front, instead of letting the assembler
    ///   write a file called `-` (the pre-fix behaviour) or fail obscurely.
    /// * several inputs: GCC refuses `-o` outright because one output cannot
    ///   hold several translation units, and for `-S -o -` concatenating them
    ///   on stdout would produce unusable output.
    fn validate_output_destination(&self) -> Result<(), String> {
        if !self.output_is_stdout() {
            return Ok(());
        }
        match self.mode {
            CompileMode::ObjectOnly => {
                Err("Cannot write an object file to standard output ('-o -'); \
                 use -S -o - for assembly or -E -o - for preprocessed source"
                    .to_string())
            }
            CompileMode::Full => Err("Cannot write an executable to standard output ('-o -'); \
                 use -S -o - for assembly or -E -o - for preprocessed source"
                .to_string()),
            CompileMode::AssemblyOnly | CompileMode::PreprocessOnly
                if self.input_files.len() > 1 =>
            {
                Err("cannot specify '-o' with '-c', '-S' or '-E' with multiple files".to_string())
            }
            _ => Ok(()),
        }
    }

    /// Run the compiler pipeline.
    pub fn run(&self) -> Result<(), String> {
        if self.input_files.is_empty() {
            return Err("No input files".to_string());
        }
        self.validate_output_destination()?;

        // Set the thread-local target pointer size for type system queries.
        // Must be done before any CType/IrType size computations.
        crate::common::types::set_target_ptr_size(self.target.ptr_size());
        crate::common::types::set_target_long_double_is_f128(matches!(
            self.target,
            Target::Aarch64 | Target::Riscv64
        ));
        // Per-ISA relocation number space (see Target::elf_machine).
        crate::common::types::set_target_elf_machine(self.target.elf_machine());
        // AUD-CPU-3: publish the invocation's code-generation permission set
        // (target + `-march` + `-m*` flags) for the intrinsic feature gate in
        // `expr_builtins`.  `effective_x86_features()` is the *same* model
        // `x86_isa()` feeds to the backend, so an intrinsic can never be
        // lowered to an instruction the emitted code is not already allowed
        // to use.  (`__builtin_cpu_supports` is a *runtime* CPUID question
        // and deliberately does NOT consult this set — see `cpu_probe`.)
        crate::common::x86_features::set_effective(self.effective_x86_features());
        crate::common::x86_features::set_explicit(self.x86_explicit_feature_bits());
        // GCC's BIGGEST_ALIGNMENT, the cap C11 `_Alignof` applies to
        // non-user-aligned types (`min_align_of_type`): 16 on x86, 32 under
        // TARGET_AVX, 64 under TARGET_AVX512F. It follows the *requested*
        // ISA (the `__AVX__`/`__AVX512F__` predefine state, same gate as
        // `set_extended_simd_macros`), not the codegen permission set, so
        // `_Alignof(v32)` is 16 by default and 32 with `-mavx` exactly like
        // GCC 16.2. Non-x86 targets keep 16 (their vector TYPE_ALIGN is
        // already capped at 16 in `vector_type_align`, so the value is moot).
        let x86_isa_width = if matches!(self.target, Target::X86_64 | Target::I686) && !self.no_sse
        {
            if self.enable_avx512f {
                64
            } else if self.enable_avx {
                32
            } else {
                16
            }
        } else {
            16
        };
        crate::common::types::set_target_biggest_alignment(x86_isa_width);
        // The psABI vector-register width of the argument classification
        // (`__m256` in one `%ymm` under `-mavx`, `__m512` in one `%zmm`
        // under `-mavx512f`, MEMORY otherwise) follows the same requested
        // ISA state, exactly like GCC's `TARGET_AVX`/`TARGET_AVX512F` and
        // Clang's `AVXLevel`. Both x86 ABIs use native wide registers
        // for explicitly enabled GNU vectors.
        crate::common::types::set_target_vector_abi_bytes(
            if matches!(self.target, Target::X86_64 | Target::I686) {
                x86_isa_width
            } else {
                16
            },
        );
        // 4-byte width-partitioned spill slots are only enabled where the
        // backend's store/load paths are proven width-consistent.  x86-64
        // narrows ≤32-bit spills; i686 naturally accesses every non-wide
        // scalar/pointer spill with one 32-bit word.
        crate::common::types::set_target_small_slots(matches!(
            self.target,
            Target::X86_64 | Target::I686
        ));

        match self.mode {
            CompileMode::PreprocessOnly => self.run_preprocess_only(),
            CompileMode::AssemblyOnly => self.run_assembly_only(),
            CompileMode::ObjectOnly => self.run_object_only(),
            CompileMode::Full => self.run_full(),
            CompileMode::SyntaxOnly => self.run_syntax_only(),
        }
    }

    // ---- Run modes ----

    fn run_preprocess_only(&self) -> Result<(), String> {
        for input_file in &self.input_files {
            if Self::is_assembly_source(input_file) || self.is_explicit_assembly() {
                // For .S files, delegate preprocessing to gcc which understands
                // assembly-specific preprocessor behavior
                self.preprocess_assembly(input_file)?;
                continue;
            }

            // -M/-MM: dependency-only mode (GCC implies -E).  Preprocess and
            // emit a real make rule: the source, every header actually
            // opened (force-includes included; __has_include probes
            // excluded), each tagged user/system so -MM applies GCC's
            // directory-based filter.  -MP adds phony rules.
            if self.dep_only {
                let source = Self::read_source(input_file)?;
                let mut preprocessor = Preprocessor::new();
                self.configure_preprocessor(&mut preprocessor);
                let filename = if input_file == "-" {
                    "<stdin>"
                } else {
                    input_file
                };
                preprocessor.set_filename(filename);
                self.process_force_includes(&mut preprocessor)?;
                let _ = preprocessor.preprocess(&source);

                // GCC exits nonzero on preprocessor errors even in -M/-MM
                // mode: a dependency file built from a failed TU would
                // under-approximate the rebuild set, so propagate like the
                // compile paths do.
                let pp_errors = preprocessor.errors();
                if !pp_errors.is_empty() {
                    for err in pp_errors {
                        eprintln!(
                            "{}:{}:{}: error: {}",
                            err.file, err.line, err.col, err.message
                        );
                    }
                    return Err(format!(
                        "{} preprocessor error(s) in {}",
                        preprocessor.errors().len().max(1),
                        filename
                    ));
                }

                let target = if let Some(ref t) = self.dep_target {
                    t.clone()
                } else {
                    // Default: derive from input filename by replacing extension with .o
                    let p = std::path::Path::new(input_file);
                    let stem = p.file_stem().unwrap_or_default().to_string_lossy();
                    format!("{}.o", stem)
                };
                let mut prereqs: Vec<String> = Vec::new();
                if input_file != "-" {
                    prereqs.push(Self::escape_dep_path(input_file));
                }
                for (path, system_dir) in preprocessor.take_dep_files() {
                    if self.dep_exclude_system && system_dir {
                        continue;
                    }
                    prereqs.push(Self::escape_dep_path(&path.to_string_lossy()));
                }
                let mut dep_text = if prereqs.is_empty() {
                    format!("{}:\n", target)
                } else {
                    format!("{}: {}\n", target, prereqs.join(" "))
                };
                if self.dep_phony {
                    for p in &prereqs {
                        dep_text.push_str(&format!("\n{}:\n", p));
                    }
                }

                if self.output_path_set {
                    // `-M -o -` prints the rules (GCC does; see the oracle in
                    // tests/regression/check_o_stdout.sh).
                    self.write_text_output(&dep_text)?;
                } else {
                    print!("{}", dep_text);
                }
                continue;
            }

            let source = Self::read_source(input_file)?;

            let mut preprocessor = Preprocessor::new();
            self.configure_preprocessor(&mut preprocessor);
            let filename = if input_file == "-" {
                "<stdin>"
            } else {
                input_file
            };
            preprocessor.set_filename(filename);
            self.process_force_includes(&mut preprocessor)?;

            if self.dump_defines {
                // -dM mode: preprocess the source (to process #define/#undef
                // directives) then dump all resulting macro definitions.
                let _ = preprocessor.preprocess(&source);

                // Check for preprocessor errors (missing includes, #error, etc.)
                let pp_errors = preprocessor.errors();
                if !pp_errors.is_empty() {
                    for err in pp_errors {
                        eprintln!(
                            "{}:{}:{}: error: {}",
                            err.file, err.line, err.col, err.message
                        );
                    }
                    return Err(format!(
                        "{} preprocessor error(s) in {}",
                        pp_errors.len(),
                        filename
                    ));
                }

                let output = preprocessor.dump_defines();
                if self.output_path_set {
                    self.write_text_output(&format!("{}\n", output))?;
                    // GCC still writes the dep rule in -dM mode when an
                    // -M-class flag is present (gcc 14.2: -E -dM -MD x.c
                    // -> x.d ruling `x.o:`); the -o value only names the
                    // derived file, never the target.
                    self.write_dep_file(
                        input_file,
                        &self.output_path,
                        &Self::dep_rule_object_target(input_file),
                    )?;
                } else {
                    println!("{}", output);
                    self.write_dep_file(
                        input_file,
                        input_file,
                        &Self::dep_rule_object_target(input_file),
                    )?;
                }
            } else {
                let preprocessed = preprocessor.preprocess(&source);
                *self.last_dep_files.borrow_mut() = preprocessor.take_dep_files();

                // Output the preprocessed text first, even if there are #error
                // directives. GCC and Clang also emit the full preprocessed
                // output before exiting non-zero, and many configure scripts
                // (e.g., privoxy) rely on grepping this output while ignoring
                // the exit code.
                let output = if self.suppress_line_markers {
                    Self::strip_line_markers(&preprocessed)
                } else {
                    preprocessed
                };

                if self.output_path_set {
                    self.write_text_output(&output)?;
                    // Write dependency file if requested (e.g., -Wp,-MMD,<depfile>).
                    // The kernel build uses this when preprocessing linker scripts
                    // (.lds.S -> .lds) and fixdep expects the .d file to exist.
                    // With `-o -` the derived name is `-.d`, exactly as GCC
                    // derives it (verified against gcc 12.2: -MD -E -o -), but
                    // the rule TARGET stays the would-be object (`x.o:`), not
                    // the dash — gcc 14.2, pinned by check_dash_output.sh row 4.
                    self.write_dep_file(
                        input_file,
                        &self.output_path,
                        &Self::dep_rule_object_target(input_file),
                    )?;
                } else {
                    print!("{}", output);
                    // GCC writes the dep rule on this path too: `-E -MF -
                    // -MMD x.c` streams `x.o: x.c` on stdout and `-E -MD`
                    // derives x.d (both measured, gcc 14.2).  Same contract
                    // as the -o arm, with input-derived names.
                    self.write_dep_file(
                        input_file,
                        input_file,
                        &Self::dep_rule_object_target(input_file),
                    )?;
                }

                // Report `#warning`-class diagnostics on stderr with the
                // source excerpt, exactly as the compile paths do (gcc
                // 16.2's `-E` prints the warning block the same way while
                // its preprocessed text still streams to stdout —
                // measured). -Werror promotion fails the run just like a
                // syntax check.
                if !preprocessor.warnings().is_empty() {
                    let mut source_manager = SourceManager::new();
                    source_manager.add_file(filename.to_string(), source.clone());
                    source_manager.build_line_map();
                    let mut diagnostics = DiagnosticEngine::new();
                    diagnostics.set_warning_config(self.effective_warning_config());
                    diagnostics.set_color_mode(self.color_mode);
                    diagnostics.set_source_manager(source_manager);
                    self.emit_preprocessor_warnings(&preprocessor, &mut diagnostics);
                    if diagnostics.has_errors() {
                        return Err(format!(
                            "{} preprocessor warning(s) promoted by -Werror in {}",
                            diagnostics.error_count(),
                            filename
                        ));
                    }
                }

                // Check for preprocessor errors (missing includes, #error,
                // etc.) AFTER emitting output so the result is still available
                // for downstream tools that ignore the exit code.
                let pp_errors = preprocessor.errors();
                if !pp_errors.is_empty() {
                    for err in pp_errors {
                        eprintln!(
                            "{}:{}:{}: error: {}",
                            err.file, err.line, err.col, err.message
                        );
                    }
                    return Err(format!(
                        "{} preprocessor error(s) in {}",
                        pp_errors.len(),
                        filename
                    ));
                }
            }
        }
        Ok(())
    }

    /// `-fsyntax-only`: run each C translation unit through preprocess +
    /// lex + parse + semantic analysis and stop. Diagnostics are reported
    /// exactly as in a real compilation (including `-Werror` promotion),
    /// but no IR is built, no assembly is generated, no object is
    /// assembled, and the link stage never runs — for any input kind.
    ///
    /// Assembly inputs have no C syntax to check; their `#include`/`#error`
    /// directives still gate through a preprocess-only pass (GCC's
    /// `-fsyntax-only` on `.S` is likewise a preprocess pass-through).
    /// Objects/archives are skipped: there is nothing left to check.
    ///
    /// This closes the WO-4 driver bug (red-team audit 2026-09-18): the
    /// flag used to be silently dropped by the unknown-argument handler,
    /// so `printf '#include <stdio.h>\n' | lccc -x c - -fsyntax-only`
    /// compiled AND LINKED the stdin TU and failed on the missing `main`.
    /// Emit collected preprocessor diagnostics (`#warning` and friends)
    /// through `diagnostics` with the [`crate::common::error::WarningKind::Cpp`]
    /// kind, so `-Wcpp`/`-Wno-cpp`/`-Werror=cpp` control them exactly as
    /// GCC does. Shared by every path that preprocesses without going
    /// through `parse_and_analyze_c_tu`, which has the same drain inline.
    fn emit_preprocessor_warnings(
        &self,
        preprocessor: &Preprocessor,
        diagnostics: &mut DiagnosticEngine,
    ) {
        for warn in preprocessor.warnings() {
            let diag = crate::common::error::Diagnostic::warning_with_kind(
                warn.message.clone(),
                crate::common::error::WarningKind::Cpp,
            )
            .with_location(&warn.file, warn.line, warn.col);
            diagnostics.emit(&diag);
        }
    }

    fn run_syntax_only(&self) -> Result<(), String> {
        for input_file in &self.input_files {
            if Self::is_object_or_archive(input_file) {
                continue;
            }
            let filename = if input_file == "-" {
                "<stdin>"
            } else {
                input_file
            };

            let source = Self::read_source(input_file)?;
            let mut preprocessor = Preprocessor::new();
            self.configure_preprocessor(&mut preprocessor);
            preprocessor.set_filename(filename);
            self.process_force_includes(&mut preprocessor)?;
            let preprocessed = preprocessor.preprocess(&source);
            *self.last_dep_files.borrow_mut() = preprocessor.take_dep_files();

            let pp_errors = preprocessor.errors();
            if !pp_errors.is_empty() {
                for err in pp_errors {
                    eprintln!(
                        "{}:{}:{}: error: {}",
                        err.file, err.line, err.col, err.message
                    );
                }
                return Err(format!(
                    "{} preprocessor error(s) in {}",
                    pp_errors.len(),
                    filename
                ));
            }

            if Self::is_assembly_source(input_file) || self.is_explicit_assembly() {
                // Nothing past the preprocessor to check for assembly.
                if self.verbose {
                    eprintln!("syntax check passed (assembly): {}", filename);
                }
                continue;
            }

            // Lex. The source manager owns the preprocessed text; the line
            // map and macro-expansion metadata give spans their
            // file:line:col resolution (same wiring as compile_to_assembly).
            let mut source_manager = SourceManager::new();
            let file_id = source_manager.add_file(input_file.to_string(), preprocessed);
            source_manager.build_line_map();
            let macro_expansions = preprocessor.take_macro_expansion_info();
            source_manager.set_macro_expansions(macro_expansions);
            let mut lexer = Lexer::new(source_manager.get_content(file_id), file_id);
            lexer.set_gnu_extensions(self.gnu_extensions);
            lexer.set_c23(self.c23_dialect());
            let tokens = lexer.tokenize();
            let lexer_diags = std::mem::take(&mut lexer.diagnostics);

            // Parse.
            let mut diagnostics = DiagnosticEngine::new();
            diagnostics.set_warning_config(self.effective_warning_config());
            diagnostics.set_color_mode(self.color_mode);
            diagnostics.set_source_manager(source_manager);
            // Drain preprocessor warnings through the engine with the Cpp
            // kind — the full compile path does this, and without it
            // `-fsyntax-only` dropped `#warning` entirely (measured against
            // gcc-16.2, which reports it in every mode, `-fsyntax-only`
            // included; `-Wcpp`/`-Wno-cpp`/`-Werror=cpp` control it here
            // for the same reason they do there).
            self.emit_preprocessor_warnings(&preprocessor, &mut diagnostics);
            for (msg, span) in lexer_diags {
                diagnostics.error(msg, span);
            }
            let mut parser = Parser::new(tokens);
            parser.set_diagnostics(diagnostics);
            parser.set_c23(self.c23_dialect());
            let ast = parser.parse();
            let diagnostics = parser.take_diagnostics();
            // Lexer diagnostics were seeded into this engine before parsing.
            // Count the complete frontend evidence, not only parser errors;
            // never run sema on a translation unit already rejected by lexing.
            if diagnostics.has_errors() {
                return Err(format!(
                    "{}: {} frontend error(s)",
                    input_file,
                    diagnostics.error_count()
                ));
            }

            // Semantic analysis.
            let mut sema = SemanticAnalyzer::new();
            sema.set_builtin_policy(self.builtin_policy.clone());
            sema.set_pre_c99_dialect(self.pre_c99_dialect());
            sema.set_weak_pragma_names(
                preprocessor
                    .weak_pragmas
                    .iter()
                    .map(|(name, _)| name.as_str()),
            );
            sema.set_diagnostics(diagnostics);
            if let Err(error_count) = sema.analyze(&ast) {
                return Err(format!("{} error(s) during semantic analysis", error_count));
            }
            let diagnostics = sema.take_diagnostics();
            // -Werror promotion applies to the syntax check too: a TU whose
            // warnings are promoted to errors must fail -fsyntax-only.
            if diagnostics.has_errors() {
                return Err(format!(
                    "{} error(s) (warnings promoted by -Werror)",
                    diagnostics.error_count()
                ));
            }
            if self.verbose && diagnostics.warning_count() > 0 {
                eprintln!("{} warning(s) generated", diagnostics.warning_count());
            }
            if self.verbose {
                eprintln!("syntax check passed: {}", filename);
            }
        }
        Ok(())
    }

    /// Preprocess an assembly file.
    ///
    /// Uses the built-in C preprocessor by default. When the `gcc_assembler`
    /// feature is enabled, falls back to GCC for preprocessing.
    fn preprocess_assembly(&self, input_file: &str) -> Result<(), String> {
        // When gcc_assembler is enabled, delegate to GCC for assembly preprocessing.
        // GCC handles certain assembly-specific preprocessing behaviors that our
        // built-in preprocessor may not fully support in all edge cases.
        #[cfg(feature = "gcc_assembler")]
        {
            self.preprocess_assembly_gcc(input_file)
        }

        // Default: use built-in preprocessor
        #[cfg(not(feature = "gcc_assembler"))]
        {
            let source = Self::read_source(input_file)?;
            let mut preprocessor = Preprocessor::new();
            self.configure_preprocessor(&mut preprocessor);
            preprocessor.define_macro("__ASSEMBLER__", "1");
            preprocessor.define_macro("_CET_H_INCLUDED", "1");
            if self.target == crate::backend::Target::X86_64 {
                preprocessor.define_macro("_CET_ENDBR", "endbr64");
            } else {
                preprocessor.define_macro("_CET_ENDBR", "endbr32");
            }
            preprocessor.define_macro("_CET_NOTRACK", "notrack");
            preprocessor.set_asm_mode(true);
            preprocessor.set_filename(input_file);
            self.process_force_includes(&mut preprocessor)
                .map_err(|e| format!("Preprocessing {} failed: {}", input_file, e))?;
            let preprocessed = preprocessor.preprocess(&source);
            *self.last_dep_files.borrow_mut() = preprocessor.take_dep_files();

            // A missing #include in a .S file is fatal (GCC aborts).  Without
            // this, header.S once preprocessed with an EMPTY voffset.h and the
            // VO_* `#if` guards silently went false.  Emit the output only when
            // preprocessing succeeded.
            let pp_errors = preprocessor.errors();
            if !pp_errors.is_empty() {
                for err in pp_errors {
                    eprintln!(
                        "{}:{}:{}: error: {}",
                        err.file, err.line, err.col, err.message
                    );
                }
                return Err(format!(
                    "{} preprocessor error(s) in {}",
                    pp_errors.len(),
                    input_file
                ));
            }

            let output = if self.suppress_line_markers {
                Self::strip_line_markers(&preprocessed)
            } else {
                preprocessed
            };

            if self.output_path_set {
                self.write_text_output(&output)?;
                // Same GCC dep contract as the C -E path: the derived .d
                // name follows the -o value (`-o -` -> `-.d`, measured), the
                // rule target stays the would-be object (`s1.o:`).
                self.write_dep_file(
                    input_file,
                    &self.output_path,
                    &Self::dep_rule_object_target(input_file),
                )?;
            } else {
                print!("{}", output);
                // No -o: GCC still writes the dep rule (`.S -E -MF -` streams
                // `s1.o: s1.S` on stdout; `.S -E -MD` derives s1.d) — same
                // input-derived names as the C -E path.
                self.write_dep_file(
                    input_file,
                    input_file,
                    &Self::dep_rule_object_target(input_file),
                )?;
            }
            Ok(())
        }
    }

    /// Preprocess an assembly file by delegating to gcc -E.
    ///
    /// Only compiled when the `gcc_assembler` feature is enabled.
    #[cfg(feature = "gcc_assembler")]
    fn preprocess_assembly_gcc(&self, input_file: &str) -> Result<(), String> {
        let config = self.target.assembler_config();
        let mut cmd = std::process::Command::new(config.command);
        cmd.args(config.extra_args);
        for path in &self.include_paths {
            cmd.arg("-I").arg(path);
        }
        for path in &self.quote_include_paths {
            cmd.arg("-iquote").arg(path);
        }
        for path in &self.isystem_include_paths {
            cmd.arg("-isystem").arg(path);
        }
        for path in &self.after_include_paths {
            cmd.arg("-idirafter").arg(path);
        }
        for def in &self.defines {
            if def.value == "1" {
                cmd.arg(format!("-D{}", def.name));
            } else {
                cmd.arg(format!("-D{}={}", def.name, def.value));
            }
        }
        for inc in &self.force_includes {
            cmd.arg("-include").arg(inc);
        }
        if self.nostdinc {
            cmd.arg("-nostdinc");
        }
        for undef in &self.undef_macros {
            cmd.arg(format!("-U{}", undef));
        }
        if self.undef_all {
            cmd.arg("-undef");
        }
        if let Some(ref lang) = self.explicit_language {
            cmd.arg("-x").arg(lang);
        }
        cmd.arg("-E");
        if self.suppress_line_markers {
            cmd.arg("-P");
        }
        if let Some(ref dep_path) = self.dep_file {
            // gcc -E owns the dep-rule format here, so mirror the driver's
            // dep state onto -Wp flags instead of post-processing:
            //  * an explicit -MF/-MMD path passes through (and `-` streams
            //    the rule to stdout inside gcc — measured: -Wp,-MMD,- ==
            //    -MF -);
            //  * a bare -MD/-MMD derives the file name exactly as gcc does
            //    (the -o value when this phase has one, else the input);
            //  * the flag class follows dep_exclude_system (-MMD vs -MD),
            //    and -MT/-MP pass through (gcc -E -MD s1.S rules `s1.o:`,
            //    includes stdc-predef.h for -MD — both measured).
            let flag_value = if dep_path.is_empty() {
                let base = if self.output_path_set {
                    self.output_path.clone()
                } else {
                    input_file.to_string()
                };
                std::path::Path::new(&base)
                    .with_extension("d")
                    .to_string_lossy()
                    .into_owned()
            } else {
                dep_path.clone()
            };
            let m_flag = if self.dep_exclude_system { "MMD" } else { "MD" };
            cmd.arg(format!("-Wp,-{},{}", m_flag, flag_value));
            if let Some(t) = self.dep_target.as_deref() {
                cmd.arg(format!("-Wp,-MT,{}", t));
            }
            if self.dep_phony {
                cmd.arg("-Wp,-MP");
            }
        }
        cmd.arg(input_file);
        if self.output_path_set {
            cmd.arg("-o").arg(&self.output_path);
        }
        let result = cmd
            .output()
            .map_err(|e| format!("Failed to preprocess {}: {}", input_file, e))?;
        if !self.output_path_set {
            print!("{}", String::from_utf8_lossy(&result.stdout));
        }
        if !result.status.success() {
            let stderr = String::from_utf8_lossy(&result.stderr);
            return Err(format!("Preprocessing {} failed: {}", input_file, stderr));
        }
        Ok(())
    }

    fn run_assembly_only(&self) -> Result<(), String> {
        for input_file in &self.input_files {
            let out_path = self.output_for_input(input_file);
            // When gcc_m16 feature is enabled, delegate -m16 C compilation to GCC
            #[cfg(feature = "gcc_m16")]
            if self.code16gcc && Self::is_c_source(input_file) {
                use super::external_tools::GccM16Mode;
                self.compile_with_gcc_m16(input_file, &out_path, GccM16Mode::Assembly)?;
                self.write_dep_file_gcc_semantics(input_file, &out_path)?;
                if self.verbose {
                    eprintln!("Assembly output (GCC -m16): {}", out_path);
                }
                continue;
            }
            // Default path: compile with internal codegen (handles .code16gcc prepend)
            let asm = self.compile_to_assembly(input_file)?;
            if self.output_path_set {
                // `-o -` streams the assembly (write_text_output is the single
                // place that knows the convention); a real path is written
                // exactly as before.
                self.write_text_output(&asm)?;
            } else {
                std::fs::write(&out_path, &asm)
                    .map_err(|e| format!("Cannot write {}: {}", out_path, e))?;
            }
            self.write_dep_file_gcc_semantics(input_file, &out_path)?;
            if self.verbose {
                eprintln!("Assembly output: {}", out_path);
            }
        }
        Ok(())
    }

    fn run_object_only(&self) -> Result<(), String> {
        for input_file in &self.input_files {
            let out_path = self.output_for_input(input_file);
            if Self::is_assembly_source(input_file) || self.is_explicit_assembly() {
                // .s/.S files (or -x assembler): pass directly to the assembler
                self.assemble_source_file(input_file, &out_path)?;
            } else {
                // When gcc_m16 feature is enabled, delegate -m16 C compilation to GCC
                #[cfg(feature = "gcc_m16")]
                if self.code16gcc {
                    use super::external_tools::GccM16Mode;
                    self.compile_with_gcc_m16(input_file, &out_path, GccM16Mode::Object)?;
                    self.write_dep_file_gcc_semantics(input_file, &out_path)?;
                    if self.verbose {
                        eprintln!("Object output (GCC -m16): {}", out_path);
                    }
                    continue;
                }
                // Default path: compile with internal codegen
                let asm = self.compile_to_assembly(input_file)?;
                let extra = self.build_asm_extra_args();
                self.target.assemble_with_extra(&asm, &out_path, &extra)?;
            }
            self.write_dep_file_gcc_semantics(input_file, &out_path)?;
            if self.verbose {
                eprintln!("Object output: {}", out_path);
            }
        }
        Ok(())
    }

    fn run_full(&self) -> Result<(), String> {
        use crate::common::temp_files::TempFile;

        // TempFile guards ensure cleanup on all exit paths (success, error, panic).
        let mut temp_guards: Vec<TempFile> = Vec::new();

        let mut extra_passthrough: Vec<String> = Vec::new();
        let mut whole_program_c_files: Vec<&str> = Vec::new();
        let can_merge_whole_program =
            self.whole_program && !self.relocatable && !self.debug_info && !self.code16gcc;

        for input_file in &self.input_files {
            if Self::is_object_or_archive(input_file) {
                // Object/archive files are passed to the linker via linker_ordered_items
                // (populated during argument parsing) to preserve their position relative
                // to -Wl, and -l flags. This is critical for --whole-archive support.
            } else if Self::is_assembly_source(input_file) || self.is_explicit_assembly() {
                // .s/.S files (or -x assembler): pass to assembler, then link
                let tmp = TempFile::try_new("ccc", Self::input_stem(input_file), "o")
                    .map_err(|error| error.to_string())?;
                self.assemble_source_file(
                    input_file,
                    tmp.to_str().map_err(|error| error.to_string())?,
                )?;
                temp_guards.push(tmp);
            } else if !Self::is_c_source(input_file) && Self::looks_like_binary_object(input_file) {
                // Unrecognized extension but file has ELF/archive magic bytes -
                // treat as object file. These weren't caught by is_object_or_archive
                // at parse time, so add them to the extra passthrough list.
                extra_passthrough.push(input_file.clone());
            } else {
                // When gcc_m16 feature is enabled, delegate -m16 C compilation to GCC
                #[cfg(feature = "gcc_m16")]
                if self.code16gcc {
                    use super::external_tools::GccM16Mode;
                    let tmp = TempFile::try_new("ccc", Self::input_stem(input_file), "o")
                        .map_err(|error| error.to_string())?;
                    self.compile_with_gcc_m16(
                        input_file,
                        tmp.to_str().map_err(|error| error.to_string())?,
                        GccM16Mode::Object,
                    )?;
                    if self.verbose {
                        eprintln!("Compiled (GCC -m16): {}", input_file);
                    }
                    self.write_dep_file(input_file, &self.output_path, &self.output_path)?;
                    temp_guards.push(tmp);
                    continue;
                }
                if can_merge_whole_program {
                    whole_program_c_files.push(input_file.as_str());
                    continue;
                }
                // Compile .c files to .o (handles .code16gcc prepend via internal codegen)
                let asm = self.compile_to_assembly(input_file)?;

                let tmp = TempFile::try_new("ccc", Self::input_stem(input_file), "o")
                    .map_err(|error| error.to_string())?;
                let extra = self.build_asm_extra_args();
                self.target.assemble_with_extra(
                    &asm,
                    tmp.to_str().map_err(|error| error.to_string())?,
                    &extra,
                )?;
                // Write dependency file for this source file. When compiling and
                // linking in one step, GCC's -Wp,-MMD uses the .o name as the
                // dependency target. We use the output executable path as target,
                // which is sufficient for kernel build's fixdep processing.
                self.write_dep_file(input_file, &self.output_path, &self.output_path)?;
                temp_guards.push(tmp);
            }
        }

        if !whole_program_c_files.is_empty() {
            let asm = if whole_program_c_files.len() == 1 {
                let single = whole_program_c_files[0];
                let asm = self.compile_to_assembly(single)?;
                self.write_dep_file(single, &self.output_path, &self.output_path)?;
                asm
            } else {
                self.compile_whole_program_c_files(&whole_program_c_files)?
            };
            let stem = Self::input_stem(whole_program_c_files[0]);
            let tmp = TempFile::try_new("ccc", stem, "o").map_err(|error| error.to_string())?;
            let extra = self.build_asm_extra_args();
            self.target.assemble_with_extra(
                &asm,
                tmp.to_str().map_err(|error| error.to_string())?,
                &extra,
            )?;
            temp_guards.push(tmp);
        }

        // Compiled temp objects go first (order relative to linker flags doesn't matter
        // for freshly compiled objects). Passthrough objects, -l flags, and -Wl, flags
        // follow in their original command-line order via linker_ordered_items.
        // Extra passthrough objects (detected by magic bytes at runtime) go after temps.
        let mut all_objects: Vec<&str> = temp_guards
            .iter()
            .map(TempFile::to_str)
            .collect::<Result<_, _>>()
            .map_err(|error| error.to_string())?;
        for obj in &extra_passthrough {
            all_objects.push(obj.as_str());
        }

        // Build linker args from -l, -L, -static flags, preserving positional ordering
        let linker_args = self.build_linker_args();

        // Emit a synthetic verbose link line for build system compatibility.
        // CMake's CMakeParseImplicitLinkInfo.cmake looks for a line matching
        // `collect2` or `ld` and extracts -L paths from it, which populate
        // CMAKE_C_IMPLICIT_LINK_DIRECTORIES (used by find_library()).
        // Without this, CMake can't find libraries in multiarch paths like
        // /usr/lib/x86_64-linux-gnu/.
        // NOTE: This line is intentionally incomplete -- it only contains -L
        // flags for CMake detection, not actual linker arguments. Our built-in
        // linker is invoked internally, not via a subprocess.
        if self.verbose {
            let lib_paths = self.target.implicit_library_paths();
            let l_flags: String = lib_paths
                .split(':')
                .filter(|p| !p.is_empty())
                .map(|p| format!(" -L{}", p))
                .collect();
            eprintln!(
                " /usr/bin/ld -dynamic-linker {} -o {}{}",
                self.target.dynamic_linker(),
                self.output_path,
                l_flags,
            );
            eprintln!("LIBRARY_PATH={}", lib_paths);
        }

        if linker_args.is_empty() {
            self.target.link(&all_objects, &self.output_path)?;
        } else {
            self.target
                .link_with_args(&all_objects, &self.output_path, &linker_args)?;
        }

        // temp_guards drop here, cleaning up all temp .o files automatically.
        // (Also cleans up on early return via ? above.)

        if self.verbose {
            eprintln!("Output: {}", self.output_path);
        }

        Ok(())
    }

    // ---- Helper methods ----

    /// Determine the output path for a given input file and mode.
    fn output_for_input(&self, input_file: &str) -> String {
        if self.output_path_set {
            return self.output_path.clone();
        }
        let stem = if input_file == "-" {
            "stdin"
        } else {
            std::path::Path::new(input_file)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("a")
        };
        match self.mode {
            CompileMode::AssemblyOnly => format!("{}.s", stem),
            CompileMode::ObjectOnly => format!("{}.o", stem),
            CompileMode::PreprocessOnly => String::new(),
            // -fsyntax-only writes nothing; `-o` is ignored (GCC behaviour).
            CompileMode::SyntaxOnly => String::new(),
            CompileMode::Full => self.output_path.clone(),
        }
    }

    /// Get a short stem name for an input file (for temp file naming).
    fn input_stem(input_file: &str) -> &str {
        if input_file == "-" {
            "stdin"
        } else {
            std::path::Path::new(input_file)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("out")
        }
    }

    /// Read a C source file, tolerating non-UTF-8 content.
    /// Valid UTF-8 files are returned as-is. Non-UTF-8 bytes (0x80-0xFF) are
    /// encoded as PUA code points (U+E080-U+E0FF) which the lexer decodes
    /// back to raw bytes inside string/character literals.
    fn read_c_source_file(path: &str) -> Result<String, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("Cannot read {}: {}", path, e))?;
        Ok(crate::common::encoding::bytes_to_string(bytes))
    }

    /// Read source from an input file. If the file is "-", reads from stdin.
    pub(super) fn read_source(input_file: &str) -> Result<String, String> {
        if input_file == "-" {
            use std::io::Read;
            let mut bytes = Vec::new();
            std::io::stdin()
                .read_to_end(&mut bytes)
                .map_err(|e| format!("Cannot read from stdin: {}", e))?;
            Ok(crate::common::encoding::bytes_to_string(bytes))
        } else {
            Self::read_c_source_file(input_file)
        }
    }

    /// Configure the preprocessor with CLI-defined macros and target.
    pub(super) fn configure_preprocessor(&self, preprocessor: &mut Preprocessor) {
        // Set target architecture macros
        match self.target {
            Target::Aarch64 => preprocessor.set_target("aarch64"),
            Target::Riscv64 => preprocessor.set_target("riscv64"),
            Target::I686 => preprocessor.set_target("i686"),
            Target::X86_64 => preprocessor.set_target("x86_64"),
        }
        // set_target may add architecture-specific default directories, so
        // clear them only after target selection. User-specified directories
        // are added separately below and remain available with -nostdinc.
        if self.nostdinc {
            preprocessor.clear_default_include_paths();
        }
        // Apply RISC-V ABI/arch overrides from -mabi= and -march= flags.
        // These must come after set_target() which sets defaults for RV64GC/lp64d.
        if self.target == Target::Riscv64 {
            if let Some(ref abi) = self.riscv_abi {
                preprocessor.set_riscv_abi(abi);
            }
            if let Some(ref march) = self.riscv_march {
                preprocessor.set_riscv_march(march);
            }
        }
        // Define __STRICT_ANSI__ for strict ISO C modes (-std=c99, -std=c11, etc.).
        // GCC defines this when non-GNU standard modes are used. Headers like
        // glibc's <features.h> and CPython's pymacro.h check for it to gate
        // GNU extensions like typeof.
        if !self.gnu_extensions {
            preprocessor.set_strict_ansi(true);
        }
        // Set inline semantics mode: -fgnu89-inline or -std=gnu89 uses GNU89
        // inline semantics (__GNUC_GNU_INLINE__), while the default C99+ mode
        // uses __GNUC_STDC_INLINE__. Projects like mpack use these macros to
        // select the correct inline linkage model.
        if self.gnu89_inline {
            preprocessor.set_gnu89_inline(true);
        }
        // __EXCEPTIONS mirrors -fexceptions (GCC: defined for C only with
        // -fexceptions; glibc stdio-lock.h branches on it).
        preprocessor.set_exceptions(self.exceptions);
        // Set optimization macros: __OPTIMIZE__ for -O1+, __OPTIMIZE_SIZE__ for -Os/-Oz.
        // The Linux kernel's BUILD_BUG() relies on __OPTIMIZE__ to expand to a noreturn
        // function call instead of a no-op.
        preprocessor.set_optimize(self.optimize, self.optimize_size);
        // GCC-compatible umbrella feature-test macro.  Individual unsafe or
        // associative-math flags permit selected transforms but do not define
        // __FAST_MATH__.
        preprocessor.set_fast_math(self.fast_math);
        // PIE and full PIC both define __PIC__; only PIE defines __PIE__.
        // Kernel `-fno-pic` clears both before RIP_REL_REF inline asm expands.
        preprocessor.set_pic(self.pic || self.pie || self.shared_lib);
        // __CET__ mirrors -fcf-protection exactly (GCC never predefines it).
        preprocessor.set_cet(self.cf_protection_value);
        preprocessor.set_ssp(self.stack_protector_mode.predefined_macro());
        preprocessor.set_pie(self.pie && !self.pic && !self.shared_lib);
        // Set SSE/SSE2/MMX predefined macros for x86 targets.
        // GCC/Clang always define __SSE__, __SSE2__, __MMX__ for x86_64 (baseline ISA).

        // i686's default baseline is the same ISA level (GCC 16.2 default
        // `-m32` = `-march=x86-64`, measured), so the baseline defines them
        // there too; an explicit pre-SSE `-march=` drops SSE/SSE2 AND __MMX__
        // (measured on GCC 16.2 / Clang 23.1.3: `-m32 -march=i686` defines
        // none of the three — but `-m32 -march=i686 -msse` revives ALL of
        // them, so MMX follows the arch's MMX or an SSE grant; see
        // `resolved_i686_mmx`), while `-mno-sse2` drops only the SSE2 pair and `-mno-sse`
        // drops SSE (MMX stays — measured on both oracles).
        // Projects like stb_image, minimp3, dr_libs use #ifdef __SSE2__ to
        // enable SIMD paths.
        let (sse_on, sse2_on, mmx_on) = if self.target == Target::I686 {
            let sse = self.resolved_i686_sse();
            (sse, self.resolved_i686_sse2(), self.resolved_i686_mmx())
        } else {
            // x86-64: the SSE pair and MMX ride the x86-64 baseline, and the
            // order-sensitive raw denial bits already carry the exact GCC
            // 16.2 matrix (measured on 16.2.0): `-mno-sse2` drops ONLY
            // __SSE2__ (and __SSE2_MATH__); `__SSE__` and `__MMX__` stay.
            // `-mno-sse` drops the SSE pair but keeps `__MMX__` (MMX is
            // independent of the vector level). A later `-msse` re-opens the
            // SSE level: its handler clears `no_sse2` only when no EXPLICIT
            // SSE2 denial is pending, so `-mno-sse -msse` restores baseline
            // SSE2 while `-mno-sse2 -msse` must not. `-mgeneral-regs-only`
            // drops all three, including `__MMX__`.
            (
                self.target == Target::X86_64 && !self.no_sse,
                self.target == Target::X86_64 && !self.no_sse2,
                self.target == Target::X86_64
                    && !self.general_regs_only
                    && !self.mmx_explicitly_disabled,
            )
        };
        preprocessor.set_sse_macros(sse_on, sse2_on, mmx_on);
        // `__FXSR__` is march-driven only: GCC 16.2 defines it for every
        // x86-64 invocation (even -mno-sse/-mgeneral-regs-only/-mno-mmx,
        // measured) and on i686 exactly for `-march=` CPUs whose row carries
        // Fxsr (pentium2+/k8 yes; pentium*/k6/athlon/via/geode/lakemont/
        // i686 no - measured against the generated table).
        let fxsr_on = self
            .fxsr_override
            .unwrap_or(self.target == Target::X86_64 || self.march_fxsr);
        preprocessor.set_fxsr_macro(fxsr_on);

        // Define extended feature macros (__SSE3__, __AVX__, __BMI__, ...).
        // The SIMD half is gated on the xmm file being open and its SSE2
        // prerequisite not being denied. A later `-march=` cannot make a
        // sticky denial over-report SSE3+ macros. The INTEGER half
        // (__BMI__, __BMI2__,
        // __LZCNT__, __POPCNT__, __MOVBE__, __RDRND__) follows the RESOLVED
        // integer state with NO -mno-sse gate: integer ISA is legal without
        // xmm state — lccc's codegen emits `andn` for `-mno-sse -O2`
        // (measured), and GCC defines `__BMI__` for
        // `-mno-sse -march=x86-64-v3` — so suppressing the macro there
        // made the preprocessor lie about the codegen contract (an
        // #ifdef __BMI__ fallback path would disagree with the emitted
        // `andnq`).
        // GFNI keeps the SSE-only gate (its intrinsics need SSE2, see the
        // AES note in `docs/x86-isa-permissions-and-simd-abi.md` §1.6);
        // SSE3+ and the AVX-dependent families require the second predicate.
        let xmm_macros_enabled = !self.no_sse;
        let simd_macros_enabled = xmm_macros_enabled && !self.no_sse2;
        preprocessor.set_extended_simd_macros(
            simd_macros_enabled && self.enable_sse3,
            simd_macros_enabled && self.enable_ssse3,
            simd_macros_enabled && self.enable_sse4_1,
            simd_macros_enabled && self.enable_sse4_2,
            simd_macros_enabled && self.enable_avx,
            simd_macros_enabled && self.enable_avx2,
            simd_macros_enabled && self.enable_aes,
            simd_macros_enabled && self.enable_pclmul,
            simd_macros_enabled && self.enable_f16c,
            simd_macros_enabled && self.enable_fma,
            // INTEGER-ISA macros follow the RESOLVED codegen permission
            // (explicit -march grant or ceiling, default x86-64 baseline,
            // -mno-* denial), not the raw request flags: __BMI__ and
            // friends must exactly mirror what the backend may emit,
            // or an #ifdef-guarded intrinsic path could disagree with
            // the scalar codegen contract.
            // `__BMI__` and `__MOVBE__` announce the CPU on i686 too, as
            // GCC's do (`-m32 -march=haswell`): lccc's BMI1 intrinsics are
            // plain C and none keys on `__MOVBE__`, so every path the
            // macros select compiles there.
            self.resolved_codegen_bmi1(),
            self.resolved_builtin_bmi2(),
            self.resolved_lzcnt(),
            self.resolved_popcnt(),
            self.resolved_isa_movbe(),
            self.enable_rdrnd,
            simd_macros_enabled && self.enable_avx512f,
            simd_macros_enabled && self.enable_avx512cd,
            simd_macros_enabled && self.enable_avx512dq,
            simd_macros_enabled && self.enable_avx512bw,
            simd_macros_enabled && self.enable_avx512vl,
            simd_macros_enabled && self.enable_avx512ifma,
            simd_macros_enabled && self.enable_avx512vbmi,
            simd_macros_enabled && self.enable_avx512vbmi2,
            simd_macros_enabled && self.enable_avx512vnni,
            simd_macros_enabled && self.enable_avx512bitalg,
            simd_macros_enabled && self.enable_avx512vpopcntdq,
            simd_macros_enabled && self.enable_avx512bf16,
            simd_macros_enabled && self.enable_avx512fp16,
            simd_macros_enabled && self.enable_avx512er,
            simd_macros_enabled && self.enable_avx512pf,
            simd_macros_enabled && self.enable_avx512vp2intersect,
            simd_macros_enabled && self.enable_avx512bmm,
            simd_macros_enabled && self.enable_avxvnni,
            simd_macros_enabled && self.enable_avxifma,
            simd_macros_enabled && self.enable_avxneconvert,
            simd_macros_enabled && self.enable_avx10_1,
            simd_macros_enabled && self.enable_avx10_2,
            xmm_macros_enabled && self.enable_gfni,
            simd_macros_enabled && self.enable_vaes,
            simd_macros_enabled && self.enable_vpclmulqdq,
            simd_macros_enabled && self.enable_avxvnniint8,
            simd_macros_enabled && self.enable_avxvnniint16,
            simd_macros_enabled && self.enable_sha512,
            simd_macros_enabled && self.enable_sm3,
            simd_macros_enabled && self.enable_sm4,
            // MOVRS, AMX and CMPCCXADD use no SSE register: GCC 16.2 keeps
            // them under `-mno-sse` and `-mgeneral-regs-only` (measured on
            // `-march=sapphirerapids`/`clearwaterforest`).
            self.enable_movrs,
            self.enable_amx_tile,
            self.enable_amx_int8,
            self.enable_amx_bf16,
            self.enable_cmpccxadd,
            self.enable_apxf,
        );
        for macro_name in self.x86_cpu_name_macros() {
            preprocessor.define_macro(macro_name, "1");
        }
        // `__PRFCHW__` mirrors the codegen permission exactly (GCC defines
        // it for `-mprfchw` and every PRFCHW `-march=`, independent of SSE).
        if self.resolved_prfchw() {
            preprocessor.define_macro("__PRFCHW__", "1");
        }
        if self.resolved_abm() {
            preprocessor.define_macro("__ABM__", "1");
        }
        // Define _REENTRANT when -pthread is used.
        // GCC and Clang automatically define _REENTRANT=1 when -pthread is passed.
        // Many configure scripts (e.g., iperf3's ax_pthread.m4) check for this macro
        // to verify that pthread support is properly configured.
        if self.pthread {
            preprocessor.define_macro("_REENTRANT", "1");
        }
        // Dialect-selected `__STDC_VERSION__`, applied BEFORE the user's
        // `-D`s so an explicit `-D__STDC_VERSION__=...` still wins.
        preprocessor.set_c23(self.c23_dialect());
        if let Some(v) = self.stdc_version {
            match v {
                Some(val) => preprocessor.define_macro("__STDC_VERSION__", val),
                // C89/C90 predate the macro; GCC defines nothing there.
                None => preprocessor.undefine_macro("__STDC_VERSION__"),
            }
        }
        // C17 6.10.8.1: a freestanding implementation defines
        // `__STDC_HOSTED__` as 0.  Before the user's `-D`s, which still win.
        if self.freestanding {
            preprocessor.define_macro("__STDC_HOSTED__", "0");
        }
        for def in &self.defines {
            preprocessor.define_macro(&def.name, &def.value);
        }
        // Disable _FORTIFY_SOURCE: glibc's fortification headers define extern
        // always_inline wrapper functions that use __builtin_va_arg_pack() and
        // __builtin_va_arg_pack_len(), which are GCC-specific constructs that
        // only work when the wrapper is inlined into the caller. Since we cannot
        // fully support these constructs, the wrappers produce incorrect code
        // (infinite recursion or wrong control flow). Undefining _FORTIFY_SOURCE
        // prevents these wrappers from being emitted.
        preprocessor.undefine_macro("_FORTIFY_SOURCE");
        // User/system include directories use the legacy LCCC_SYSROOT overlay:
        // prefixed paths are preferred when present, with a host fallback.
        // This differs from GNU --sysroot and is not an isolation boundary.
        // See `backend/common.rs` and `predefined_macros.rs`.
        fn sysroot_include_path(path: &str) -> String {
            // Use the same overlay accessor as the built-in linker. GNU driver
            // sysroot flags are rejected separately until fully implemented.
            let prefixed = match crate::backend::common::sysroot() {
                Some(root) => format!("{root}{path}"),
                None => return path.to_string(),
            };
            if std::path::Path::new(&prefixed).is_dir() {
                prefixed
            } else {
                path.to_string()
            }
        }
        for path in &self.include_paths {
            preprocessor.add_include_path(&sysroot_include_path(path));
        }
        for path in &self.quote_include_paths {
            preprocessor.add_quote_include_path(&sysroot_include_path(path));
        }
        for path in &self.isystem_include_paths {
            preprocessor.add_system_include_path(&sysroot_include_path(path));
        }
        for path in &self.after_include_paths {
            preprocessor.add_after_include_path(&sysroot_include_path(path));
        }
    }

    /// Process force-included files (-include flag) through the preprocessor before
    /// the main source. Matches GCC's behavior: `-include file` acts as if
    /// `#include "file"` appeared at the top of the primary source file.
    /// The file is searched in this order:
    /// 1. Current working directory (for relative paths)
    /// 2. Include paths (-I, -isystem, system defaults, -idirafter)
    pub(super) fn process_force_includes(
        &self,
        preprocessor: &mut Preprocessor,
    ) -> Result<(), String> {
        for path in &self.force_includes {
            let (resolved, origin) = if std::path::Path::new(path).is_absolute() {
                (std::path::PathBuf::from(path), None)
            } else {
                // GCC first searches CWD for -include; a file found there
                // has no search-chain slot, even if a later -I names CWD.
                let cwd_path = std::env::current_dir()
                    .map(|cwd| cwd.join(path))
                    .unwrap_or_else(|_| std::path::PathBuf::from(path));
                if cwd_path.is_file() {
                    (cwd_path, None)
                } else {
                    match preprocessor.resolve_force_include_with_origin(path) {
                        Some(found) => (found.path, found.origin),
                        None => (cwd_path, None),
                    }
                }
            };

            let content = Self::read_c_source_file(&resolved.to_string_lossy())
                .map_err(|e| format!("{}: {}", path, e))?;
            preprocessor.preprocess_force_include_with_origin(
                &content,
                &resolved.to_string_lossy(),
                origin,
            );
        }
        Ok(())
    }

    // ---- Core compilation pipeline ----

    pub(super) fn effective_ubsan_trap(&self) -> crate::ir::lowering::ubsan::UbsanFlags {
        self.sanitize_enabled.intersection(self.sanitize_trap)
    }

    pub(super) fn effective_wrap_signed_overflow(&self) -> bool {
        use crate::ir::lowering::ubsan::UbsanFlags;
        self.wrap_signed_overflow
            || self.trapv
            || !self
                .effective_ubsan_trap()
                .intersection(UbsanFlags::SIGNED_OVERFLOW.union(UbsanFlags::SHIFT_BASE))
                .is_empty()
    }

    pub(super) fn resolved_zero_call_used_regs_fp_simd(&self) -> bool {
        match self.target {
            Target::X86_64 => !self.no_sse && !self.general_regs_only,
            Target::I686 => self.resolved_i686_sse() && !self.general_regs_only,
            Target::Aarch64 => !self.general_regs_only,
            Target::Riscv64 => self.riscv_abi.as_deref() != Some("lp64") && !self.general_regs_only,
        }
    }

    fn init_compilation_session(
        &self,
    ) -> (
        std::sync::Arc<crate::backend::regalloc::RaConfig>,
        crate::backend::x86::cpu_model::X86Tune,
        bool,
    ) {
        // Capture RA/codegen bisection policy once, before this compilation
        // reaches any function-level backend path.  The Arc is threaded via
        // CodegenOptions rather than re-reading process environment there.
        let ra_config = std::sync::Arc::new(crate::backend::regalloc::RaConfig::from_process_env());

        // Clear the "this translation unit reads a runtime CPU probe word"
        // latch before lowering; the codegen tail below appends the probe
        // helper it latches (AUD-CPU-1).
        crate::common::cpu_probe::reset_runtime_needed();

        // Resolve the microarchitectural tuning row once per compilation.
        // `-mtune` wins over `-march`, `native` goes through CPUID; both
        // the middle end (process-global accessor) and the backend
        // (`CodegenOptions::tune`) see the same row.
        let tune = crate::backend::x86::cpu_model::resolve(
            self.x86_march.as_deref(),
            self.x86_tune.as_deref(),
        );
        crate::backend::x86::cpu_model::set_active(tune);
        if let Ok(v) = std::env::var("LCCC_DUMP_TUNE") {
            if v == "all" {
                for cpu in crate::backend::x86::cpu_model::X86Cpu::ALL {
                    eprintln!("{}", cpu.tune().dump());
                }
            } else if !v.is_empty() && v != "0" {
                eprint!("{}", tune.dump());
            }
        }
        if self.verbose {
            eprintln!("lccc: note: x86 tuning row '{}'", tune.name);
        }
        let time_phases = std::env::var("CCC_TIME_PHASES").is_ok();
        (ra_config, tune, time_phases)
    }

    fn compile_to_assembly(&self, input_file: &str) -> Result<String, String> {
        let (ra_config, tune, time_phases) = self.init_compilation_session();
        let t0 = std::time::Instant::now();
        let (mut module, source_manager, fp_environment_observable) =
            self.lower_c_tu_to_ir(input_file, time_phases)?;
        if self.whole_program && !self.relocatable {
            crate::ir::whole_program::internalize_module(&mut module, false);
        }
        self.compile_ir_module_to_assembly(
            module,
            source_manager.as_deref(),
            input_file,
            fp_environment_observable,
            ra_config,
            tune,
            time_phases,
            t0,
        )
    }

    fn compile_whole_program_c_files(&self, input_files: &[&str]) -> Result<String, String> {
        let (ra_config, tune, time_phases) = self.init_compilation_session();
        let t0 = std::time::Instant::now();
        let mut modules = Vec::with_capacity(input_files.len());
        let mut any_fp_observable = false;
        for &input_file in input_files {
            let (module, _sm, fp_obs) = self.lower_c_tu_to_ir(input_file, time_phases)?;
            any_fp_observable |= fp_obs;
            self.write_dep_file(input_file, &self.output_path, &self.output_path)?;
            modules.push(module);
        }
        let mut merged = crate::ir::whole_program::merge_modules(modules)?;
        crate::ir::whole_program::internalize_module(&mut merged, false);
        self.compile_ir_module_to_assembly(
            merged,
            None,
            input_files[0],
            any_fp_observable,
            ra_config,
            tune,
            time_phases,
            t0,
        )
    }

    fn parse_and_analyze_c_tu(
        &self,
        input_file: &str,
        time_phases: bool,
    ) -> Result<
        (
            crate::frontend::parser::ast::TranslationUnit,
            crate::frontend::sema::analysis::SemaResult,
            DiagnosticEngine,
            Vec<(String, Option<String>)>,
            Vec<(String, String)>,
            bool,
        ),
        String,
    > {
        let source = Self::read_source(input_file)?;
        let t0 = std::time::Instant::now();

        // Preprocess
        let mut preprocessor = Preprocessor::new();
        self.configure_preprocessor(&mut preprocessor);
        let filename = if input_file == "-" {
            "<stdin>"
        } else {
            input_file
        };
        preprocessor.set_filename(filename);
        self.process_force_includes(&mut preprocessor)?;
        let preprocessed = preprocessor.preprocess(&source);
        let fp_environment_observable =
            self.fp_trapping_math || self.fp_rounding_math || preprocessor.fenv_access_requested;
        *self.last_dep_files.borrow_mut() = preprocessor.take_dep_files();
        if time_phases {
            eprintln!("[TIME] preprocess: {:.3}s", t0.elapsed().as_secs_f64());
        }

        // Create diagnostic engine for structured error/warning reporting
        let mut diagnostics = DiagnosticEngine::new();
        diagnostics.set_warning_config(self.effective_warning_config());
        diagnostics.set_color_mode(self.color_mode);

        // Emit preprocessor warnings through diagnostic engine with Cpp kind
        // so they can be controlled via -Wcpp / -Wno-cpp / -Werror=cpp.
        // Preprocessor diagnostics carry file:line:col location info for GCC-compatible output.
        for warn in preprocessor.warnings() {
            let diag = crate::common::error::Diagnostic::warning_with_kind(
                warn.message.clone(),
                crate::common::error::WarningKind::Cpp,
            )
            .with_location(&warn.file, warn.line, warn.col);
            diagnostics.emit(&diag);
        }

        // Check for #error directives and missing #include errors
        let pp_errors = preprocessor.errors();
        if !pp_errors.is_empty() {
            for err in pp_errors {
                let diag = crate::common::error::Diagnostic::error(err.message.clone())
                    .with_location(&err.file, err.line, err.col);
                diagnostics.emit(&diag);
            }
            return Err(format!(
                "{} preprocessor error(s) in {}",
                pp_errors.len(),
                input_file
            ));
        }

        // Lex
        let t1 = std::time::Instant::now();
        let mut source_manager = SourceManager::new();
        // Move preprocessed output into source manager (avoids cloning the entire string).
        // Then borrow a reference back for the lexer.
        let file_id = source_manager.add_file(input_file.to_string(), preprocessed);
        // Build line map from preprocessor line markers for source location tracking.
        // Uses stored content from add_file() and reuses already-computed line offsets.
        source_manager.build_line_map();
        // Transfer macro expansion metadata from preprocessor to source manager
        // for "in expansion of macro 'X'" diagnostic notes.
        let macro_expansions = preprocessor.take_macro_expansion_info();
        source_manager.set_macro_expansions(macro_expansions);
        let mut lexer = Lexer::new(source_manager.get_content(file_id), file_id);
        lexer.set_gnu_extensions(self.gnu_extensions);
        lexer.set_c23(self.c23_dialect());
        let tokens = lexer.tokenize();
        let lexer_diags = std::mem::take(&mut lexer.diagnostics);
        if time_phases {
            eprintln!(
                "[TIME] lex: {:.3}s ({} tokens)",
                t1.elapsed().as_secs_f64(),
                tokens.len()
            );
        }

        if self.verbose {
            eprintln!("Lexed {} tokens from {}", tokens.len(), input_file);
        }

        // Parse -- the diagnostic engine holds the source manager for span
        // resolution and snippet rendering. It's also set on the parser for
        // backward-compatible span_to_location() calls.
        let t2 = std::time::Instant::now();
        diagnostics.set_source_manager(source_manager);
        // Surface lexer diagnostics after the source manager is attached so
        // spans resolve to file:line:col.
        for (msg, span) in lexer_diags {
            diagnostics.error(msg, span);
        }
        let mut parser = Parser::new(tokens);
        parser.set_diagnostics(diagnostics);
        parser.set_c23(self.c23_dialect());
        // Code generation runs the post-optimization asm-constraint validator,
        // which can prove local constant initializers and inlined arguments
        // that the parser alone cannot classify. Syntax-only mode deliberately
        // leaves this disabled above.
        parser.set_deferred_asm_immediate_validation(true);
        let ast = parser.parse();
        if time_phases {
            eprintln!("[TIME] parse: {:.3}s", t2.elapsed().as_secs_f64());
        }

        if parser.error_count > 0 {
            return Err(format!(
                "{}: {} parse error(s)",
                input_file, parser.error_count
            ));
        }

        // Retrieve diagnostic engine (which holds the source manager) for subsequent phases
        let diagnostics = parser.take_diagnostics();

        if self.verbose {
            eprintln!("Parsed {} declarations", ast.decls.len());
        }

        // Semantic analysis -- pass diagnostic engine to sema (still holds
        // the source manager so sema diagnostics can resolve spans)
        let t3 = std::time::Instant::now();
        let mut sema = SemanticAnalyzer::new();
        sema.set_builtin_policy(self.builtin_policy.clone());
        sema.set_pre_c99_dialect(self.pre_c99_dialect());
        sema.set_weak_pragma_names(
            preprocessor
                .weak_pragmas
                .iter()
                .map(|(name, _)| name.as_str()),
        );
        sema.set_diagnostics(diagnostics);
        if let Err(error_count) = sema.analyze(&ast) {
            // Errors already emitted through diagnostic engine with source spans
            return Err(format!("{} error(s) during semantic analysis", error_count));
        }
        let diagnostics = sema.take_diagnostics();
        let sema_result = sema.into_result();
        if time_phases {
            eprintln!("[TIME] sema: {:.3}s", t3.elapsed().as_secs_f64());
        }

        // Check for warnings promoted to errors by -Werror / -Werror=<name>.
        // The sema pass may have returned Ok (no hard errors), but the diagnostic
        // engine may have accumulated promoted-warning-errors that should stop compilation.
        if diagnostics.has_errors() {
            return Err(format!(
                "{} error(s) (warnings promoted by -Werror)",
                diagnostics.error_count()
            ));
        }

        // Log diagnostic summary if there were any warnings
        if self.verbose && diagnostics.warning_count() > 0 {
            eprintln!("{} warning(s) generated", diagnostics.warning_count());
        }

        Ok((
            ast,
            sema_result,
            diagnostics,
            preprocessor.weak_pragmas,
            preprocessor.redefine_extname_pragmas,
            fp_environment_observable,
        ))
    }

    fn lower_c_tu_to_ir(
        &self,
        input_file: &str,
        time_phases: bool,
    ) -> Result<
        (
            crate::ir::module::IrModule,
            Option<std::rc::Rc<SourceManager>>,
            bool,
        ),
        String,
    > {
        let (
            ast,
            sema_result,
            diagnostics,
            weak_pragmas,
            redefine_extname_pragmas,
            fp_environment_observable,
        ) = self.parse_and_analyze_c_tu(input_file, time_phases)?;

        // Lower to IR (target-aware for ABI-specific lowering decisions)
        // Pass sema's TypeContext, function signatures, and expression type annotations
        // to the lowerer so it has pre-populated type info upfront.
        let t4 = std::time::Instant::now();
        // Packed lowering ceiling for GCC vector-extension arithmetic. The
        // frontend lowers a whole `vector_size` operation to the x86-64
        // `Vec*` families when the code-generation ISA has the register
        // (`x86_isa()`: SSE2 `%xmm` always on x86-64, `%ymm` under the
        // AVX2 ceiling) and the optimizer runs (the -O0 contract keeps the
        // per-lane scalar form, which is the differential oracle of the
        // packed path in tests/regression/check_vector_ext_codegen.sh).
        // Publishing it here, before `Lowerer::lower`, mirrors the ABI
        // width above and is reset per TU like every other target cell.
        {
            let isa = self.x86_isa();
            // i686 participates at the SSE2 ceiling: the default -m32
            // baseline (`-march=x86-64` minus the x86-64 bit) has SSE2, and
            // `resolved_i686_sse2` denies it for pre-SSE -march rows and the
            // -mno-sse* spellings, exactly like the rest of the i686 SIMD
            // permissions.  The ceiling stops at 16 bytes: the i686 Vec*
            // emitters cover the 128-bit image, while 256-bit packed
            // lowering (and the optimizer's vectorizer permission below)
            // stays x86-64-only, so 32-byte shapes keep their per-lane
            // scalar fallback on -m32.  The -O0 contract is unchanged: the
            // scalar form remains the differential oracle of the packed path.
            let bytes = if self.opt_level == 0 {
                0
            } else if self.target == Target::X86_64 && isa.simd {
                if isa.ymm { 32 } else { 16 }
            } else if self.target == Target::I686 && self.resolved_i686_sse2() {
                16
            } else {
                0
            };
            crate::common::types::set_target_vector_lowering_bytes(bytes);
            crate::passes::vectorize::set_x86_simd_isa(
                isa.simd,
                isa.ymm,
                isa.sse41,
                self.target == Target::X86_64
                    && self.enable_avx512f
                    && self.enable_avx512vl
                    && !self.no_sse,
            );
        }
        crate::passes::vectorize::set_x86_sse1_available(
            self.target == Target::X86_64 && !self.no_sse && !self.general_regs_only,
        );
        let lowerer = Lowerer::with_type_context(
            self.target,
            sema_result.type_context,
            sema_result.functions,
            sema_result.expr_types,
            sema_result.const_values,
            diagnostics,
            self.gnu89_inline,
        )
        .with_pass_policy(crate::common::pass_policy::PassPolicy::new(
            self.effective_wrap_signed_overflow(),
            fp_environment_observable,
        ))
        .with_wrapv(self.wrap_signed_overflow)
        .with_trapv(self.trapv)
        .with_ubsan_trap(self.effective_ubsan_trap())
        .with_trivial_auto_var_init(self.trivial_auto_var_init)
        .with_default_visibility(self.default_visibility.clone())
        .with_builtin_policy(self.builtin_policy.clone())
        .with_enumerator_refs(sema_result.enumerator_refs)
        .with_local_address_returns(sema_result.local_address_returns);
        let (mut module, mut diagnostics) = lowerer.lower(&ast);
        // IR lowering is a user-facing phase: it rejects ill-typed builtins,
        // target-specific intrinsics and unsupported constructs, and every one
        // of those diagnostics carries a span. The engine therefore keeps the
        // SourceManager *through* lowering — without it `render_diagnostic`
        // silently drops the `file:line:col` prefix and the caret snippet, and
        // the user gets a location-less error. The debug-info phase below is
        // the only consumer that needs the manager back, so take it once
        // lowering has returned the engine.
        let source_manager = diagnostics.take_source_manager();

        // Apply #pragma weak directives from the preprocessor.
        for (symbol, target) in &weak_pragmas {
            if let Some(alias_target) = target {
                // #pragma weak symbol = alias -> create weak alias
                module
                    .aliases
                    .push((symbol.clone(), alias_target.clone(), true));
            } else {
                // #pragma weak symbol -> mark as weak
                module.symbol_attrs.push((symbol.clone(), true, None));
            }
        }

        // Apply #pragma redefine_extname directives from the preprocessor.
        // TODO: This uses .set aliases which works when both symbols are defined
        // locally, but a proper implementation would rename symbol references
        // during lowering/codegen for the case where new_name is external.
        for (old_name, new_name) in &redefine_extname_pragmas {
            module
                .aliases
                .push((old_name.clone(), new_name.clone(), false));
        }
        // The builtin policy needs the whole TU -- a definition can follow its
        // first call, and the pragma aliases above define symbols too -- so
        // it is published onto the IR here, after the last symbol is known
        // and before any pass runs.
        crate::passes::builtin_policy::apply(&mut module, &self.builtin_policy);

        // Apply -fcommon: mark tentative definitions as COMMON symbols.
        // A tentative definition is a global variable at file scope with no initializer
        // and no extern/static storage class. With -fcommon, these use COMMON linkage
        // so the linker merges duplicates across TUs instead of reporting errors.
        if self.fcommon {
            for global in &mut module.globals {
                if !global.is_common
                    // `__attribute__((nocommon))` opts this object out of
                    // common linkage even though -fcommon is in force.
                    && !global.is_nocommon
                    && !global.is_extern
                    && !global.is_static
                    && !global.is_thread_local
                    && matches!(global.init, crate::ir::module::GlobalInit::Zero)
                {
                    global.is_common = true;
                }
            }
        }

        if time_phases {
            eprintln!(
                "[TIME] lowering: {:.3}s ({} functions)",
                t4.elapsed().as_secs_f64(),
                module.functions.len()
            );
        }

        // Check for errors emitted during lowering (e.g., unresolved types, invalid constructs)
        if diagnostics.has_errors() {
            return Err(format!(
                "{} error(s) during IR lowering",
                diagnostics.error_count()
            ));
        }

        // Log diagnostic summary if there were any warnings during lowering
        if self.verbose && diagnostics.warning_count() > 0 {
            eprintln!("{} warning(s) during lowering", diagnostics.warning_count());
        }

        if self.verbose {
            eprintln!("Lowered to {} IR functions", module.functions.len());
        }

        Ok((module, source_manager, fp_environment_observable))
    }

    #[expect(clippy::too_many_arguments)]
    fn compile_ir_module_to_assembly(
        &self,
        module: crate::ir::module::IrModule,
        source_manager: Option<&SourceManager>,
        input_file: &str,
        fp_environment_observable: bool,
        ra_config: std::sync::Arc<crate::backend::regalloc::RaConfig>,
        tune: crate::backend::x86::cpu_model::X86Tune,
        time_phases: bool,
        t0: std::time::Instant,
    ) -> Result<String, String> {
        let (module, pgo_unit) = self.optimize_ir_module(
            module,
            input_file,
            fp_environment_observable,
            ra_config.as_ref(),
            time_phases,
        );
        // Immediate-only asm constraints may depend on a local constant or
        // an inline parameter that is not resolvable during parsing. Recheck
        // the optimized IR here. A Value is not an immediate; emitting the
        // backend's historical zero placeholder would silently miscompile a
        // dynamic operand (e.g. `.long %c0`). Dead-static elimination removes
        // an unused static always_inline template after its callers have been
        // inlined, but deliberately retains an address-taken definition as a
        // callable function-pointer identity. Every surviving body must
        // therefore be checked; an always_inline flag is not proof that the
        // body is unemittable.
        for func in &module.functions {
            if func.is_declaration {
                continue;
            }
            for block in &func.blocks {
                for (inst_idx, inst) in block.instructions.iter().enumerate() {
                    if let crate::ir::reexports::Instruction::InlineAsm {
                        inputs,
                        input_symbols,
                        ..
                    } = inst
                    {
                        for (index, (constraint, operand, _)) in inputs.iter().enumerate() {
                            if crate::backend::inline_asm::constraint_is_immediate_only(constraint)
                                && matches!(operand, crate::ir::reexports::Operand::Value(_))
                                && input_symbols.get(index).and_then(Option::as_ref).is_none()
                            {
                                let span = block
                                    .source_spans
                                    .get(inst_idx)
                                    .copied()
                                    .filter(|span| *span != crate::common::source::Span::dummy())
                                    .unwrap_or(func.body_span);
                                let location = source_manager
                                    .map(|sm| sm.resolve_span(span))
                                    .map(|loc| {
                                        format!("{}:{}:{}: ", loc.file, loc.line, loc.column)
                                    })
                                    .unwrap_or_else(|| format!("{input_file}:1:1: "));
                                return Err(format!(
                                    "{location}non-constant operand for immediate-only inline asm constraint `{constraint}` remains after optimization in function `{}`",
                                    func.name
                                ));
                            }
                        }
                    }
                }
            }
        }
        let mut module = self.split_and_layout_blocks(module, time_phases);
        self.renumber_module_labels(&mut module);
        let module = self.finalize_module_for_codegen(module, &pgo_unit);
        self.emit_target_assembly(
            &module,
            source_manager,
            input_file,
            fp_environment_observable,
            ra_config,
            tune,
            time_phases,
            t0,
        )
    }

    fn optimize_ir_module(
        &self,
        mut module: crate::ir::module::IrModule,
        input_file: &str,
        fp_environment_observable: bool,
        ra_config: &crate::backend::regalloc::RaConfig,
        time_phases: bool,
    ) -> (crate::ir::module::IrModule, String) {
        // PGO use side: load the profile directory (once per process —
        // per-TU lookups are unit-keyed) and activate it for THIS unit before
        // any optimization pass runs, so PGO-guided inlining, loop unrolling
        // and branch profiling see real counts. (Before this change, this path was
        // never connected: `-fprofile-use` compiled plain code, byte-identical
        // to `-O3` — verified on zlib-ng minigzip.)
        let pgo_use_dir = self
            .pgo_use
            .clone()
            .or_else(|| std::env::var("LCCC_PGO_USE").ok());
        if pgo_use_dir.is_some() {
            crate::pgo::init_pgo_profile(pgo_use_dir.as_deref());
            let u0 = crate::pgo::profile::unit_identity(input_file);
            crate::pgo::prepass_activate(&u0);
            // Build the data-driven profile summary (percentile hot/cold
            // thresholds over the unit's count distribution — LLVM
            // ProfileSummaryInfo analogue) once per unit; inliner, unroller
            // and layout all consume it.
            if let Some(p) = crate::pgo::get_pgo_profile() {
                crate::pgo::summary::set_summary(crate::pgo::summary::build_for_unit(p, &u0));
            }
        }

        // GCC's __builtin_setjmp creates a returns-twice edge that is not an
        // ordinary IR CFG edge. Keep automatic objects in memory in such a
        // function: promoting them to one-way SSA lets the normal path's value
        // dominate the resume path and drops stores that builtin_longjmp must
        // observe (gcc.c-torture pr60003). `semantic_volatile` stays false —
        // this is an SSA barrier, not a source-level volatile access.
        for func in &mut module.functions {
            let has_builtin_setjmp = func.blocks.iter().any(|block| {
                block.instructions.iter().any(|inst| {
                    matches!(
                        inst,
                        crate::ir::reexports::Instruction::Intrinsic {
                            op: crate::ir::intrinsics::IntrinsicOp::BuiltinSetjmp,
                            ..
                        }
                    )
                })
            });
            if has_builtin_setjmp {
                for block in &mut func.blocks {
                    for inst in &mut block.instructions {
                        if let crate::ir::reexports::Instruction::Alloca { volatile, .. } = inst {
                            *volatile = true;
                        }
                    }
                }
            }
        }

        // Run optimization passes
        let t5 = std::time::Instant::now();
        promote_allocas(&mut module);
        if time_phases {
            eprintln!("[TIME] mem2reg: {:.3}s", t5.elapsed().as_secs_f64());
        }

        // Profile identity. The PRE-pass fingerprint (right after mem2reg,
        // deterministic and profile-independent in both generate and use
        // builds) is the profile key; the POST-pass fingerprint (right after
        // run_passes) detects CFG drift from profile-guided transforms.
        let pgo_active = self.pgo_use.is_some()
            || std::env::var("LCCC_PGO_USE").is_ok()
            || self.pgo_generate.is_some()
            || std::env::var("LCCC_PGO_GENERATE").is_ok();
        if pgo_active {
            let pgo_unit = crate::pgo::profile::unit_identity(input_file);
            let mut pre = crate::common::fx_hash::FxHashMap::default();
            for f in &module.functions {
                if !f.is_declaration && !f.blocks.is_empty() {
                    pre.insert(
                        f.name.clone(),
                        crate::pgo::profile::cfg_fingerprint(&f.name, &pgo_unit, f),
                    );
                }
            }
            crate::pgo::stash_pre_hashes(pre);
            let mut post = crate::common::fx_hash::FxHashMap::default();
            for f in &module.functions {
                if !f.is_declaration && !f.blocks.is_empty() {
                    post.insert(
                        f.name.clone(),
                        crate::pgo::profile::cfg_fingerprint(&f.name, &pgo_unit, f),
                    );
                }
            }
            crate::pgo::stash_post_hashes(post);
        }

        let t6 = std::time::Instant::now();
        // S13: -finstrument-functions. Runs BEFORE the optimizer so
        // the hook calls exist at every optimization level and are
        // register-allocated like any other call (GCC instruments at the
        // same semantic point: after prologue, before every return).
        if self.instrument_functions {
            crate::passes::instrument::run(&mut module);
        }
        // If PGO use is active, run PGO layout before passes? Actually after, but we prepare
        run_passes(
            &mut module,
            self.opt_level,
            self.target,
            crate::passes::loop_idiom::Capabilities {
                native_popcount: self.resolved_fast_popcount(),
                i686_sse: self.target == Target::I686
                    && !self.codegen_no_sse()
                    && !self.general_regs_only,
            },
            self.code16gcc,
            self.effective_wrap_signed_overflow(),
            fp_environment_observable,
            self.fp_reassoc,
            self.resolved_fp_contract(),
            self.math_errno,
            // Code-generation ISA permission — one struct shared with the
            // backend (see backend::x86::isa for the policy). i686 has no
            // baseline SSE2 and its vector lowerings are x86-64 shapes, so it
            // never vectorizes (verified: 0 xmm refs at -O2/-O3, with and
            // without -ffast-math).
            self.x86_isa(),
            self.target == Target::X86_64 && !self.no_sse && !self.general_regs_only,
            self.target == Target::X86_64
                && self.enable_avx
                && !self.no_sse
                && !self.general_regs_only,
            self.enable_fma,
            // Scalar BMI1 (ANDN) for the middle end's CH-fold defer —
            // mirrors CodegenOptions::bmi1 for the x86-64 backend (the
            // i686 backend has no andn fusion, so it stays false there).
            // Resolved permission: granted only by an explicit `-mbmi` or a
            // `-march=` profile that includes it; `-mno-bmi` wins.
            // resolved_bmi1() is X86_64-locked, so the fold defers exactly
            // when the backend can fuse (i686 never defers — its backend
            // cannot emit ANDN).
            self.resolved_bmi1(),
            // AVX-512VL (F base implied, GCC semantics) for the ARX
            // vectorizers' rotate-shape choice — mirrors
            // CodegenOptions::avx512vl.  Under `-mno-sse` there is no
            // vector lowering at all, so the gate fails closed.
            self.target == Target::X86_64
                && self.enable_avx512f
                && self.enable_avx512vl
                && !self.no_sse,
            self.disable_tree_vectorize,
            ra_config,
            // `-fPIC`/`-shared` units keep calls to their own external
            // definitions (semantic interposition, GCC -fPIC default).
            self.pic || self.shared_lib,
            self.tls_model_floor,
        );
        if time_phases {
            eprintln!("[TIME] opt passes: {:.3}s", t6.elapsed().as_secs_f64());
        }

        // Post-pass fingerprint snapshot (the CFG drift detector). The
        // pre-pass snapshot above was taken BEFORE optimization; this one is
        // taken after, at the same pipeline point in generate and use builds
        // (before promotion/instrumentation touch the CFG).
        if pgo_active {
            let pgo_unit = crate::pgo::profile::unit_identity(input_file);
            let mut post = crate::common::fx_hash::FxHashMap::default();
            for f in &module.functions {
                if !f.is_declaration && !f.blocks.is_empty() {
                    post.insert(
                        f.name.clone(),
                        crate::pgo::profile::cfg_fingerprint(&f.name, &pgo_unit, f),
                    );
                }
            }
            crate::pgo::stash_post_hashes(post);
        }

        // Indirect-call value-profile promotion (use mode). Runs after
        // the main pass loop but BEFORE phi elimination, so SSA phis are
        // still valid; the re-inlined hot targets then go through the
        // regular pipeline (split ranges, phi elimination, codegen).
        let pgo_unit = crate::pgo::profile::unit_identity(input_file);
        if self.pgo_use.is_some() || std::env::var("LCCC_PGO_USE").is_ok() {
            let nprom = crate::pgo::promote::promote_indirect_calls(&mut module, &pgo_unit);
            if nprom > 0 {
                let _ = crate::passes::inline::run(&mut module);
            }
        }

        // Pressure-guided live-range shrinkage runs after PGO promotion and
        // every ordinary SSA transform, but before call/global splitters and
        // phi elimination create non-SSA copy webs.  The pass itself is the
        // profitability gate: below-threshold and non-improving regions keep
        // their original order.
        if self.live_range_shrinkage
            && self.opt_level >= 1
            && std::env::var_os("CCC_NO_LIVE_RANGE_SHRINKAGE").is_none()
        {
            let changed = crate::passes::live_range_shrinkage::run(&mut module, self.target);
            if changed != 0 {
                crate::passes::verify::verify_after_pass(&module, "live_range_shrinkage");
            }
        }

        // IPA-PTA is a whole-unit, fail-closed analysis.  It runs after
        // inlining/PGO promotion so direct and indirect call targets are as
        // precise as the final IR permits, and before post-SSA splitters
        // create position-dependent copy webs.
        if self.ipa_pta && self.opt_level >= 1 && std::env::var_os("CCC_NO_IPA_PTA").is_none() {
            let changed = crate::passes::ipa_pta::run(&mut module, self.pic || self.shared_lib);
            if changed != 0 {
                crate::passes::verify::verify_after_pass(&module, "ipa_pta");
            }
        }

        // x86-64 hardware shifts mask their counts; drop the C-level masks
        // that only feed such counts. This must follow every semantic IR
        // pass (see backend::x86::shift_count for the contract).
        if self.target == Target::X86_64
            && self.opt_level >= 1
            && std::env::var_os("CCC_NO_SHIFT_COUNT_DEMAND").is_none()
        {
            for func in &mut module.functions {
                crate::backend::x86::shift_count::run(func);
            }
        }

        // x86-64 sibling calls in branch arms: give each call block that
        // ends in `Branch(R)` the shared return block's `Return`, so the
        // backend's per-block sibling-call shape matches (see
        // backend::x86::ret_dup). Last CFG change before the splitters;
        // after the PGO post-pass fingerprint, so generate and use builds
        // hash the same CFG. `CCC_NO_SIBCALL` turns the conversion off, and
        // with it this rewrite (a duplicated epilogue would be its only
        // effect); `CCC_DISABLE_PASSES=ret_dup` turns off only the rewrite.
        if self.target == Target::X86_64
            && self.opt_level >= 1
            && std::env::var_os("CCC_NO_SIBCALL").is_none()
            && !crate::passes::pass_disabled(
                std::env::var("CCC_DISABLE_PASSES").unwrap_or_default(),
                "ret_dup",
            )
        {
            let mut changed = 0;
            for func in &mut module.functions {
                changed += crate::backend::x86::ret_dup::run(func);
            }
            if changed != 0 {
                crate::passes::verify::verify_after_pass(&module, "ret_dup");
            }
        }

        // Dynamic-TLS addresses that are calls on this target (AArch64
        // descriptors) must be call points to the splitters' liveness too;
        // same policy as the backend's (`CodegenOptions::pic`, floor).
        crate::common::tls::install_call_sequence_symbols(&crate::common::tls::model_table(
            &module,
            crate::common::tls::TlsPolicy {
                shlib: self.pic || self.shared_lib,
                floor: self.tls_model_floor,
            },
        ));
        (module, pgo_unit)
    }

    /// Fold phi-only latch relays into their headers so each incoming edge
    /// gets one parallel copy (see `passes::phi_relay_fold`).  Runs right
    /// before phi elimination, which is what turns each relay into copies.
    fn fold_phi_relays(&self, module: &mut crate::ir::module::IrModule) {
        if self.opt_level < 2
            || crate::passes::pass_disabled(
                std::env::var("CCC_DISABLE_PASSES").unwrap_or_default(),
                "phi_relay_fold",
            )
        {
            return;
        }
        for func in module.functions.iter_mut() {
            if !func.is_declaration && !func.blocks.is_empty() {
                crate::passes::phi_relay_fold::run(func);
            }
        }
        crate::passes::verify::verify_after_pass(module, "phi_relay_fold");
    }

    fn split_and_layout_blocks(
        &self,
        mut module: crate::ir::module::IrModule,
        time_phases: bool,
    ) -> crate::ir::module::IrModule {
        // Live range splitting: split call-spanning values, then clean up
        if std::env::var("CCC_NO_SPLIT_RANGES").is_err() {
            let max_splits = std::env::var("CCC_SPLIT_MAX")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(30);
            // Which functions the global location allocator split: RA-06 is
            // skipped for exactly those (it subsumes the intra-block pass for
            // the same function; a module-wide flag skipped it everywhere).
            let nfuncs = module.functions.len();
            // Each function's value-id bound before any splitter ran: ids at
            // or above it are the splitters' own (see
            // `propagate_copies_sparing`).
            let fresh_floor: Vec<u32> = module
                .functions
                .iter()
                .map(|f| f.max_value_id().saturating_add(1))
                .collect();
            for func in &mut module.functions {
                if !func.is_declaration && func.blocks.len() > 10 {
                    crate::backend::split_ranges::split_call_spanning_ranges(func, max_splits);
                }
            }
            // P0-A global location allocation: cross-block Belady-MIN
            // spill gaps, immutable capture slots with reload-at-next-use,
            // and source-less rematerialization. Default ON; the master gate
            // `CCC_RA_GLOBAL_LOCATION` is an emergency kill switch
            // (0/off/no/false disables). Only source-less rematerialization
            // is enabled by the shipped policy (spill gaps stay off by
            // default, see GlaPolicy). This is the cross-block successor of
            // the intra-block RA-06 splitter below; when it runs it
            // subsumes the intra-block pass for the same function (double
            // splitting would pay for both stores).
            let mut global_location_split = vec![false; nfuncs];
            if crate::backend::location_alloc::gate_enabled() {
                let gmax = crate::backend::location_alloc::max_splits_from_env();
                let tls_symbols: crate::common::fx_hash::FxHashSet<String> = module
                    .globals
                    .iter()
                    .filter(|g| g.is_thread_local)
                    .map(|g| g.name.clone())
                    .collect();
                for (fi, func) in module.functions.iter_mut().enumerate() {
                    if !func.is_declaration && !func.blocks.is_empty() {
                        let n = crate::backend::location_alloc::run(
                            func,
                            gmax,
                            self.opt_level,
                            &tls_symbols,
                        );
                        global_location_split[fi] = n > 0;
                    }
                }
            }
            // RA-06: pressure-driven reload-at-next-use. Unlike the call
            // splitter above, this targets CALL-FREE high-pressure blocks —
            // the shape the measurement showed the call splitter can never
            // reach (`CCC_SPLIT_MAX=200` moved exactly zero counters on the
            // benchmark corpus because the loops that spill contain no
            // calls). Opt-in until the A/B census says otherwise. Skipped
            // when the global location allocator already split a function.
            if std::env::var("CCC_PRESSURE_SPLIT").is_ok() {
                let pmax = std::env::var("CCC_PRESSURE_SPLIT_MAX")
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(64);
                for (fi, func) in module.functions.iter_mut().enumerate() {
                    if !func.is_declaration && !func.blocks.is_empty() && !global_location_split[fi]
                    {
                        crate::backend::split_ranges::split_high_pressure_ranges(func, pmax);
                    }
                }
            }
            // Loop-transparent splitting (spill values live across a hot loop
            // but unused inside it). NOTE: measured as a net loss on the
            // benchmark suite — the store/reload round-trips cost more than the
            // freed registers save, because the values competing for registers
            // in hot loops are the in-loop ones (bases/IVs), which can't be
            // transparent-split. Kept opt-in for reference.
            if std::env::var("CCC_LOOP_SPLIT").is_ok() {
                for func in &mut module.functions {
                    if !func.is_declaration && !func.blocks.is_empty() {
                        crate::backend::split_ranges::split_loop_transparent_ranges(
                            func, max_splits,
                        );
                    }
                }
            }
            // Final cleanup of EVERY function, split or not: copy propagation
            // + DCE remove the splitters' Store/Load -> Copy chains and, at
            // -O0 (no IR pipeline), the lowering's temporaries. It used to run
            // module-wide only when some function split, so a function's code
            // depended on its neighbours (measured: -O0 x86-64 2942 / i686
            // 3060 corpus instructions between the two scopes). Deliberate
            // constant homes the hoisting passes created (opt >= 2: the -O1
            // tier returns before fp/int constant hoisting) are spared:
            // forwarding them re-sank movabsq magic multipliers and, on i686,
            // x87 F64 constants (a stack-temporary rebuild per use) into loops.
            let int_model = crate::passes::int_const_hoist::model_for(self.target);
            let hoists_ran = self.opt_level >= 2;
            for (func, &floor) in module.functions.iter_mut().zip(&fresh_floor) {
                if !func.is_declaration && !func.blocks.is_empty() {
                    let int_homes = match int_model {
                        Some(m) if hoists_ran => {
                            crate::passes::int_const_hoist::const_homes(func, m)
                        }
                        _ => Default::default(),
                    };
                    let spare =
                        |v: crate::ir::reexports::Value, c: &crate::ir::reexports::IrConst| {
                            hoists_ran
                                && (crate::passes::fp_const_hoist::is_hoisted_fp_const(c)
                                    || int_homes.contains(&v.0))
                        };
                    crate::passes::copy_prop::propagate_copies_sparing(func, floor, &spare);
                    crate::passes::dce::eliminate_dead_code(func);
                }
            }
        }

        self.fold_phi_relays(&mut module);

        // Lower SSA phi nodes to copies before codegen
        if std::env::var_os("CCC_VALIDATE_SSA").is_some() {
            crate::passes::validate_unique_defs(&module, "backend:pre-eliminate_phis");
        }
        let t7 = std::time::Instant::now();
        // i686's 6-free-GPR economy cannot hold the back-edge-spanning
        // ranges the cycle-accurate phi resolver creates; it defaults to
        // the legacy resolver (measured -7.4% sha256_transform if forced).
        let narrow_gprs = self.target.ptr_size() == 4;
        eliminate_phis(&mut module, narrow_gprs);
        if std::env::var("CCC_DUMP_IR_PHI").is_ok() {
            eprintln!("==== IR after eliminate_phis ====");
            eprintln!("{:#?}", module);
            eprintln!("==== END IR after eliminate_phis ====");
        }
        if std::env::var_os("CCC_VALIDATE_SSA").is_some() {
            crate::passes::validate_unique_defs(&module, "backend:eliminate_phis");
        }
        if time_phases {
            eprintln!("[TIME] phi elimination: {:.3}s", t7.elapsed().as_secs_f64());
        }

        // ms178: phi elimination is the LAST IR transform, so the copy-backs it
        // introduces have never been optimized: every phi copy-back becomes a
        // store→load→store relay in codegen (gzip's longest_match had 120 copies
        // surviving to codegen). Clean them up with the sound post-phi pass
        // (dead-copy elimination + same-block-after propagation through
        // single-def values), then DCE the now-dead instructions.
        if std::env::var("CCC_NO_PHICLEANUP").is_err() {
            let t7b = std::time::Instant::now();
            for func in &mut module.functions {
                if !func.is_declaration && !func.blocks.is_empty() {
                    crate::passes::copy_prop::propagate_copies_post_phi(func);
                    crate::passes::dce::eliminate_dead_code(func);
                }
            }
            if time_phases {
                eprintln!(
                    "[TIME] post-phi cleanup: {:.3}s",
                    t7b.elapsed().as_secs_f64()
                );
            }
        }

        // Congruent recurrence-web merging: phi elimination lowers
        // loop-carried phis to copy webs, and structurally identical phis
        // (same seed, same increment, same seed/latch blocks) produce webs
        // that hold the SAME value at every point — three webs for
        // linux_rbtree's single `&node_pool[i]`, each an independent RA
        // recurrence paying its own slot round-trip. Merge them into one
        // before the backend's liveness/RA sees the web set. Runs after the
        // post-phi copy cleanup (which is calibrated on the original
        // shapes) and before the edge-copy layout.
        if std::env::var("CCC_NO_WEB_CONGRUENCE").is_err() {
            let t7c = std::time::Instant::now();
            let mut merged_webs = 0usize;
            for func in &mut module.functions {
                if !func.is_declaration && !func.blocks.is_empty() {
                    let n = crate::passes::web_congruence::merge_congruent_recurrence_webs(func);
                    if n > 0 {
                        merged_webs += n;
                        crate::passes::dce::eliminate_dead_code(func);
                    }
                }
            }
            if time_phases && merged_webs > 0 {
                eprintln!(
                    "[TIME] web congruence: {:.3}s ({} webs merged)",
                    t7c.elapsed().as_secs_f64(),
                    merged_webs
                );
            }
        }

        // W5: phi elimination appends mutually-exclusive edge-copy blocks at
        // function end. Place each beside its sole predecessor so contiguous
        // live intervals reflect the real CFG lifetime rather than overlapping
        // every intervening branch arm. Enabled after the full gzip/expat,
        // regression, and differential-fuzz campaign; retain a diagnostic kill
        // switch for downstream bisection.
        if self.target == Target::X86_64
            && self.opt_level >= 2
            && std::env::var("CCC_NO_EDGE_COPY_LAYOUT").is_err()
        {
            for func in &mut module.functions {
                if !func.is_declaration {
                    crate::backend::split_ranges::place_edge_copy_blocks(func);
                }
            }
        }

        // Final block layout: optimization passes and phi elimination append
        // new blocks (vector bodies, trampolines, exit blocks) at the end of
        // `func.blocks`, which scrambles the linearized order the backend's
        // liveness/regalloc relies on. Re-layout in reverse post-order so loop
        // bodies and their exit blocks stay contiguous and loop-carried values
        // don't falsely span calls.
        // Bottom-test counted loops BEFORE laying blocks out: inversion
        // changes the CFG (the back edge moves from the latch to the body
        // entry), and the layout pass recomputes loops from scratch, so it
        // sees and lays out the rotated shape.
        // Both switches are resolved once for the whole run, here.  Reading them
        // inside the pass made this loop do two `environ` scans per function, and
        // left the module's own documentation ("resolved once, by the driver")
        // describing an intention the call site had not implemented.
        let loop_invert_cfg = crate::passes::loop_invert::LoopInvertConfig::new(
            std::env::var("CCC_NO_LOOP_INVERT").is_err(),
            std::env::var("CCC_DEBUG_LOOP_INVERT").is_ok(),
            // The duplicated bottom test is an affine exit-compare producer,
            // and CCC_NO_AFFINE_EXIT_FOLD owns EVERY producer of that fold.
            std::env::var("CCC_NO_AFFINE_EXIT_FOLD").is_err(),
            std::env::var("CCC_DEBUG_AFFINE_FOLD").is_ok(),
            std::env::var("CCC_NO_LOOP_DOWNCOUNT").is_err(),
        );
        for func in &mut module.functions {
            if !func.is_declaration {
                crate::passes::loop_invert::invert_loops_with(func, loop_invert_cfg);
            }
        }

        if std::env::var("CCC_NO_BLOCK_RELAYOUT").is_err() {
            // Layout start order is optimization-level aware. The throughput
            // pipeline (-O1..-O3) runs the static chain layout
            // (`relayout_blocks_static_chain`): greedy trace construction
            // with structural edge weights, which keeps loop bodies
            // contiguous AND places the estimated-hot successor as the
            // fall-through (the historical `Existing` start preserved
            // append-order, which on control-flow-heavy functions leaves
            // every block transition an explicit `jmp`). Size-optimized
            // builds (-Os/-Oz — which includes all -m16 real-mode setup code
            // via the size policy) keep the reverse-post-order start:
            // mutually branching blocks stay adjacent, maximizing short rel8
            // branches. Measured ~570 B of .text on the linux-cachymod setup
            // corpus, which is the margin that keeps _end under the 32 KiB
            // gate (see block_layout::relayout_blocks_loop_aware_rpo).
            //
            // -O0 ships the chain order without the dual-layout exploration
            // (two extra emissions plus two peephole runs per reordered
            // function). Measured on four binutils 2.47 TUs (gas expr.c,
            // read.c, tc-i386.c; bfd elflink.c): identical instruction counts
            // and equal or smaller .text, at 35-40% less compile time. -O1
            // and up keep it: there it buys 0.2-0.6% .text.
            let dual_layout = if self.opt_level == 0 {
                crate::passes::block_layout::DualLayout::ChainOnly
            } else {
                crate::passes::block_layout::DualLayout::Explore
            };
            for func in &mut module.functions {
                if !func.is_declaration {
                    if self.optimize_size {
                        crate::passes::block_layout::relayout_blocks_loop_aware_rpo(func);
                    } else {
                        crate::passes::block_layout::relayout_blocks_static_chain(
                            func,
                            dual_layout,
                        );
                    }
                }
            }
        }
        module
    }

    fn renumber_module_labels(&self, module: &mut crate::ir::module::IrModule) {
        // CRITICAL FIX: Renumber all block labels to ensure global uniqueness across all functions
        // This fixes a bug where optimization passes can create duplicate labels
        {
            use crate::common::fx_hash::FxHashMap;
            use crate::ir::instruction::BlockId;
            let mut next_global_label = 0u32;
            // The renumber map also translates the promoted-hot block
            // labels recorded by the value-profiling promotion pass (which
            // runs before this pass); layout reads them AFTER renumbering.
            let mut renumber_map: FxHashMap<u32, u32> = FxHashMap::default();

            for func in &mut module.functions {
                if func.is_declaration {
                    continue;
                }

                // Build a map from old labels to new globally-unique labels
                let mut label_map: FxHashMap<BlockId, BlockId> = FxHashMap::default();
                for block in &func.blocks {
                    if !label_map.contains_key(&block.label) {
                        label_map.insert(block.label, BlockId(next_global_label));
                        renumber_map.insert(block.label.0, next_global_label);
                        next_global_label += 1;
                    }
                }

                // Renumber all block labels
                for block in &mut func.blocks {
                    block.label = label_map[&block.label];
                }
                func.terminator_spans
                    .remap(|label| label_map.get(&label).copied());

                // Renumber label references held INSIDE instructions.
                //
                // Terminators are not the only place a BlockId appears:
                //
                //  * `InlineAsm::goto_labels` -- `asm goto` targets. Missing
                //    this left the asm template pointing at a pre-renumber id,
                //    so `%l[l_yes]` expanded to `.LBB6` while the block was
                //    emitted as `.LBB2`. The label was referenced but never
                //    defined; the assembler then produced a relocation against
                //    a null symbol and objtool rejected the object
                //    ("special: can't find new instruction"). Every kernel
                //    static-branch site (`__jump_table`) hits this.
                //  * `Phi::incoming` -- the predecessor label of each value.
                //  * `LabelAddr` -- `&&label` computed-goto addresses.
                //  * `global_init_label_blocks` -- `&&label` in static data.
                for block in &mut func.blocks {
                    use crate::ir::reexports::Instruction;
                    for inst in &mut block.instructions {
                        match inst {
                            Instruction::InlineAsm { goto_labels, .. } => {
                                for (_, target) in goto_labels.iter_mut() {
                                    if let Some(&new) = label_map.get(target) {
                                        *target = new;
                                    }
                                }
                            }
                            Instruction::Phi { incoming, .. } => {
                                for (_, pred) in incoming.iter_mut() {
                                    if let Some(&new) = label_map.get(pred) {
                                        *pred = new;
                                    }
                                }
                            }
                            Instruction::LabelAddr { label, .. } => {
                                if let Some(&new) = label_map.get(label) {
                                    *label = new;
                                }
                            }
                            _ => {}
                        }
                    }
                }
                for target in &mut func.global_init_label_blocks {
                    if let Some(&new) = label_map.get(target) {
                        *target = new;
                    }
                }

                // Renumber all terminator targets
                for block in &mut func.blocks {
                    use crate::ir::reexports::Terminator;
                    match &mut block.terminator {
                        Terminator::Branch(target) => {
                            if let Some(&new) = label_map.get(target) {
                                *target = new;
                            }
                        }
                        Terminator::CondBranch {
                            true_label,
                            false_label,
                            ..
                        } => {
                            if let Some(&new) = label_map.get(true_label) {
                                *true_label = new;
                            }
                            if let Some(&new) = label_map.get(false_label) {
                                *false_label = new;
                            }
                        }
                        Terminator::Switch { cases, default, .. } => {
                            for (_val, target) in cases {
                                if let Some(&new) = label_map.get(target) {
                                    *target = new;
                                }
                            }
                            if let Some(&new) = label_map.get(default) {
                                *default = new;
                            }
                        }
                        Terminator::IndirectBranch {
                            possible_targets, ..
                        } => {
                            for target in possible_targets {
                                if let Some(&new) = label_map.get(target) {
                                    *target = new;
                                }
                            }
                        }
                        Terminator::Return(_) | Terminator::Unreachable => {}
                    }
                }
            }
            crate::pgo::remap_promoted_hot(&renumber_map);

            // Rewrite `.LBB<n>` names inside static initializer DATA: the
            // renumbering above updated every BlockId reference, but
            // `GlobalInit::GlobalAddr` stores the label NAME as a string
            // (`static void *t[] = { &&lbl };` lowered to `.quad .LBBn`).
            // Without this rewrite the data pointed at the OLD label id
            // while the function emitted the renumbered one — the computed
            // goto jumped to a wrong/nonexistent block (gcc.c-torture
            // 20071220-1: `.quad .LBB3` while `addr:` was emitted as
            // `.LBB2`).
            fn rewrite_init_labels(
                init: &mut crate::ir::module::GlobalInit,
                map: &FxHashMap<u32, u32>,
            ) {
                use crate::ir::module::GlobalInit;
                match init {
                    GlobalInit::GlobalAddr(name) => {
                        if let Some(num) = name.strip_prefix(".LBB") {
                            if let Ok(old) = num.parse::<u32>() {
                                if let Some(&new) = map.get(&old) {
                                    *name = format!(".LBB{}", new);
                                }
                            }
                        }
                    }
                    GlobalInit::Compound(items) => {
                        for element in items.iter_mut() {
                            rewrite_init_labels(element, map);
                        }
                    }
                    _ => {}
                }
            }
            for global in &mut module.globals {
                rewrite_init_labels(&mut global.init, &renumber_map);
            }
        }
    }

    fn finalize_module_for_codegen(
        &self,
        mut module: crate::ir::module::IrModule,
        pgo_unit: &str,
    ) -> crate::ir::module::IrModule {
        // PGO is post-optimization and post-label-renumber; profiles are
        // keyed by the PRE-pass fingerprint (stable across gen/use).
        if self.pgo_use.is_some() || std::env::var("LCCC_PGO_USE").is_ok() {
            crate::pgo::propagate_profile(&mut module, pgo_unit);
        }
        if let Some(ref pgo_dir) = self.pgo_generate {
            crate::pgo::instrument::instrument_module(
                &mut module,
                pgo_dir,
                pgo_unit,
                self.pgo_update.as_deref(),
                self.target,
                &crate::pgo::pre_hashes(),
                &crate::pgo::post_hashes(),
            );
        } else if let Ok(dir) = std::env::var("LCCC_PGO_GENERATE") {
            crate::pgo::instrument::instrument_module(
                &mut module,
                &dir,
                pgo_unit,
                None,
                self.target,
                &crate::pgo::pre_hashes(),
                &crate::pgo::post_hashes(),
            );
        }
        if let Some(profile) = crate::pgo::get_pgo_profile() {
            crate::pgo::layout::layout_module(&mut module, profile, pgo_unit);
        }

        // Hot-loop alignment is decided here: post-optimization,
        // post-label-renumber, post-PGO-layout — the block order is now the
        // emission order. The recorded directives are consumed by codegen
        // immediately before each block label. With a profile in use, the
        // PGO hotness map recorded by pgo::layout gates which loops pad.
        // The tight-loop policy's final size bucket is resolved by the
        // integrated x86 assembler from exact encoded lengths. It must not
        // be enabled when the emitted assembly leaves the process (`-S`) or
        // goes to an external assembler (the `gcc_assembler` feature, or a
        // non-x86 target): the private `.lccc_tight_loop` directive is only
        // understood by lccc's own parser.
        let integrated_assembler = cfg!(not(feature = "gcc_assembler"))
            && !matches!(self.mode, CompileMode::AssemblyOnly)
            && matches!(
                self.target,
                crate::backend::Target::X86_64 | crate::backend::Target::I686
            );
        crate::passes::loop_align::align_module(
            &module,
            &crate::passes::loop_align::LoopAlignConfig {
                opt_level: self.opt_level,
                size_opt: self.optimize_size,
                pgo_active: crate::pgo::get_pgo_profile().is_some(),
                loops: self.align_loops,
                jumps: self.align_jumps,
                integrated_assembler,
            },
        );
        if std::env::var_os("LCCC_NO_SCHEDULE").is_none()
            && std::env::var_os("LCCC_SCHEDULE").is_some()
        {}

        // Debug: dump IR labels before codegen
        if std::env::var_os("CCC_VALIDATE_SSA").is_some() {
            crate::passes::validate_unique_defs(&module, "backend:pre-codegen");
        }
        if std::env::var("LCCC_DUMP_PRECG").is_ok() {
            eprintln!("==== IR immediately before codegen ====");
            eprintln!("{:#?}", module);
            eprintln!("==== END IR before codegen ====");
        }
        if std::env::var("LCCC_DEBUG_LABELS").is_ok() {
            for func in &module.functions {
                eprintln!(
                    "[IR] Function {}: labels = {:?}",
                    func.name,
                    func.blocks.iter().map(|b| b.label.0).collect::<Vec<_>>()
                );
            }
        }
        // `LCCC_DUMP_IR`: the final IR, one instruction per line (documented
        // debug knob, engineering/agent/RULES.md; read once per module).
        if std::env::var("LCCC_DUMP_IR").is_ok() {
            for func in &module.functions {
                if func.is_declaration {
                    continue;
                }
                eprintln!("=== IR {} ===", func.name);
                for (bi, b) in func.blocks.iter().enumerate() {
                    eprintln!("  block {} (label {}):", bi, b.label.0);
                    for inst in &b.instructions {
                        eprintln!("    {:?}", inst);
                    }
                    eprintln!("    term: {:?}", b.terminator);
                }
            }
        }

        // Note: we intentionally do NOT run copy_prop after phi elimination.
        // The IR is no longer in SSA form at this point - Copy instructions from
        // phi elimination represent moves at specific program points. Propagating
        // through them can change semantics (reading a value before it's defined
        // in the current iteration of a loop). Stack size reduction is handled
        // by copy coalescing in codegen instead.

        // Backend IR-integrity gate (runs at EVERY opt level): no block may
        // reach codegen without a predecessor path from the entry block.
        // A nested short-circuit fold inside the frontend's recursive
        // lower_condition_branch (fold_comparison_pair: `(x==0)||(x!=0)`
        // -> true) terminates the current block with an unconditional
        // branch while the outer `||` arm's rhs_label block has already
        // been created and is lowered into afterwards. The stranded block
        // is unreachable and its operands may reference values whose
        // definitions were removed together with the folded branch
        // (gcc.c-torture 20000314-1 at -O0: the `*(char *) winds` arm kept
        // a Load whose pointer value lost its def; the x86 backend's
        // session-26 hard gate — correctly — refuses to materialise a
        // value with no home and the compile ICE'd). -O0 skips the pass
        // pipeline entirely, so cfg_simplify's copy of this logic never
        // ran there; late -O1+ transforms (bool_thread, phi elimination)
        // can in principle strand blocks after the last simplify_cfg
        // round, so the gate runs unconditionally right before codegen.
        // Unreachable code has no observable semantics — removal is
        // always sound and touches no reachable instruction.
        crate::passes::cfg_simplify::for_each_function_eliminate_unreachable(&mut module);
        crate::backend::stack_protector::instrument_module(
            &mut module,
            self.target,
            self.stack_protector_mode,
            self.ssp_buffer_size,
            &self.stack_protector_guard,
        );
        module
    }

    /// Enforce the final-text contracts on the emitted assembly: the ISA the
    /// target grants (`-march=`/`-m` flags, each function's own
    /// `target("...")`) and debug-info honesty. Every emitter (codegen,
    /// MachInst, peephole) has run, so this one choke point sees each
    /// instruction the object will contain; a refused instruction names its
    /// function instead of emitting code that would #UD.
    fn validate_target_assembly(
        &self,
        module: &crate::ir::module::IrModule,
        asm: &str,
        input_file: &str,
    ) -> Result<(), String> {
        // `-mno-sse` / `-mgeneral-regs-only` (x86-64): the xmm register file
        // is architecturally off-limits — the kernel runs with CR4.OSFXSR
        // clear and every SSE instruction is #UD.  lccc's FP lowering has no
        // x87 fallback on x86-64, so any floating-point VALUE that reaches
        // codegen in such a TU produces SSE code.  GCC refuses these TUs
        // ("SSE register return/argument with SSE disabled"); silently
        // emitting `movsd` into a -mno-sse kernel object is a boot-time #UD.
        // The final text is the one choke point that sees every emitter
        // (codegen, MachInst, peephole), so the check is made here.
        if self.target == Target::X86_64 && (self.no_sse || self.general_regs_only) {
            if let Some((func, line)) = first_sse_reference(asm) {
                return Err(format!(
                    "{}: in function '{}': floating-point operation requires SSE, \
                     but SSE is disabled (-mno-sse / -mgeneral-regs-only); \
                     x86-64 lccc has no x87 lowering for it (offending instruction: `{}`)",
                    input_file,
                    func,
                    line.trim()
                ));
            }
        }
        // SSE1 and SSE2 permissions are independent. Keep legal scalar SSE1
        // available after an SSE2 denial, but fail closed on emitters that
        // still require SSE2 (notably F64: there is no x87 fallback here).
        if self.target == Target::X86_64 && self.no_sse2 && !self.no_sse {
            // A local legacy-vector grant keeps the same XMM ABI as the
            // SSE1 TU baseline. Check the actual function scope, not a TU-wide
            // exemption that could hide an illegal neighboring function.
            let sse2_functions: std::collections::HashSet<&str> = module
                .functions
                .iter()
                .filter(|f| {
                    !f.target_features.is_empty()
                        && crate::backend::x86::isa::function_feature_bits(
                            self.effective_x86_features().raw(),
                            self.x86_explicit_feature_bits(),
                            &f.target_features,
                            true,
                        ) & (1 << crate::common::x86_features::X86Feature::Sse2 as u8)
                            != 0
                })
                .map(|f| f.name.as_str())
                .collect();
            if let Some((func, line)) =
                first_sse2_reference_unless(asm, |name| sse2_functions.contains(name))
            {
                return Err(format!(
                    "{input_file}: in function '{func}': operation requires SSE2, \
                     but SSE2 is disabled (-mno-sse2); x86-64 lccc has no \
                     SSE1/x87 lowering for it (offending instruction: `{}`)",
                    line.trim()
                ));
            }
        }
        // A function's own `__attribute__((target("...")))` can deny what the
        // TU allows. x86-64 codegen applies the denial to its vector ISA but
        // still lowers FP values through SSE (and long double through x87),
        // and i686 ignores per-function targets altogether, so the same
        // final-text choke point enforces them per function. GCC 16.2
        // refuses such functions ("SSE register return with SSE disabled",
        // "x87 register return with x87 disabled").
        if matches!(self.target, Target::X86_64 | Target::I686) {
            if let Some((func, line, unit, attr)) = first_target_attribute_leak(
                module,
                asm,
                self.target == Target::I686,
                self.effective_x86_features().raw(),
                self.x86_explicit_feature_bits(),
            ) {
                return Err(format!(
                    "{input_file}: in function '{func}': {unit} is disabled by \
                     __attribute__((target(\"{attr}\"))) and lccc has no lowering of this \
                     operation without it (offending instruction: `{}`)",
                    line.trim()
                ));
            }
        }
        // i686 has x87 lowerings for every scalar FP operation, so the
        // resolved ISA is a hard contract: a leak of a higher SSE level is a
        // backend bug that would be #UD on the silicon the user targeted
        // (`-march=i686`, `-mno-sse*`, or the default `-m32` = x86-64-v1,
        // which has SSE2 but not SSE3). Fail closed instead.
        if self.target == Target::I686 {
            // Keyed on the resolved ISA, not on codegen policy: a later
            // grant revives a level after `-mgeneral-regs-only` (GCC
            // semantics), and user intrinsics may then use it even though
            // lccc's own scalar lowering conservatively stays on x87.
            // A function's own target() grant lifts the floor for that
            // function only (GCC accepts `target("sse2")` under `-mno-sse2`);
            // its neighbours stay checked, and denials remain the job of
            // first_target_attribute_leak above.
            let grants: std::collections::HashMap<&str, u64> = module
                .functions
                .iter()
                .filter(|f| !f.target_features.is_empty())
                .map(|f| {
                    let bits = crate::backend::x86::isa::function_feature_bits(
                        self.effective_x86_features().raw(),
                        self.x86_explicit_feature_bits(),
                        &f.target_features,
                        false,
                    );
                    (f.name.as_str(), bits)
                })
                .collect();
            let granted = |func: &str, level: usize| {
                use crate::common::x86_features::X86Feature as F;
                let feature = [F::Sse, F::Sse2, F::Sse3][level];
                grants
                    .get(func)
                    .is_some_and(|bits| bits & (1 << feature as u8) != 0)
            };
            let leak = (!self.resolved_i686_sse())
                .then(|| first_xmm_reference_if(asm, |func, _| !granted(func, 0)))
                .flatten()
                .map(|hit| (hit, "SSE"))
                .or_else(|| {
                    (!self.resolved_i686_sse2())
                        .then(|| first_sse2_reference_unless(asm, |func| granted(func, 1)))
                        .flatten()
                        .map(|hit| (hit, "SSE2"))
                })
                .or_else(|| {
                    (!self.resolved_i686_sse3())
                        .then(|| first_sse3_reference_unless(asm, |func| granted(func, 2)))
                        .flatten()
                        .map(|hit| (hit, "SSE3"))
                });
            if let Some(((func, line), level)) = leak {
                return Err(format!(
                    "{input_file}: in function '{func}': instruction requires {level}, \
                     which the target ISA (-march=/-m flags) does not include \
                     (offending instruction: `{}`)",
                    line.trim()
                ));
            }
        }
        // x87 permission (GCC's MASK_80387). Every x87 instruction lccc
        // emits carries an FP value (64-bit atomics use cmpxchg8b), so
        // integer-only translation units — the kernel's `-msoft-float` /
        // `-mgeneral-regs-only` objects — are unaffected, while FP code is
        // refused instead of silently using a disabled register file.
        if matches!(self.target, Target::I686 | Target::X86_64) && self.no_x87 {
            if let Some((func, line)) = first_x87_reference(asm) {
                return Err(format!(
                    "{input_file}: in function '{func}': floating-point operation requires \
                     the x87 FPU, which is disabled (-mno-80387 / -msoft-float / \
                     -mgeneral-regs-only); lccc has no soft-float lowering \
                     (offending instruction: `{}`)",
                    line.trim()
                ));
            }
        }
        if matches!(self.target, Target::I686 | Target::X86_64) && self.no_fp_ret_in_387 {
            if let Some((func, what)) =
                first_x87_return_boundary(module, asm, self.target == Target::I686)
            {
                return Err(format!(
                    "{input_file}: in function '{func}': {what} uses the x87 \
                     return register, which -mno-fp-ret-in-387 disables; lccc does \
                     not implement the general-register FP return convention"
                ));
            }
        }
        if self.target == Target::Aarch64 && self.general_regs_only {
            if let Some((func, line)) = first_arm_simd_reference(asm) {
                return Err(format!(
                    "{input_file}: in function '{func}': FP/SIMD register use is forbidden \
                     by -mgeneral-regs-only (offending instruction: `{}`)",
                    line.trim()
                ));
            }
        }
        // Debug-info honesty law: `-g` must never be silently dropped.
        // lccc's debug support today is line tables (.file/.loc →
        // .debug_line); it does not emit .debug_info (DIEs/types), and
        // optimization may erase every source span a TU was lowered with,
        // leaving objects with NO debug sections while -g was requested.
        // Downstream consumers (the kernel build's pahole/BTF step,
        // debuggers, addr2line) must be able to trust the toolchain to say
        // so instead of failing later with an unrelated error — the same
        // never-silently-wrong law that guards port-I/O and SSE lowering.
        if self.debug_info && !asm_contains_debug_records(asm) {
            eprintln!(
                "{}: warning: -g requested but no debug information was \
                 generated for this translation unit (lccc emits line tables \
                 only; .debug_info/type info is not implemented)",
                input_file
            );
        }
        Ok(())
    }

    #[expect(clippy::too_many_arguments)]
    fn emit_target_assembly(
        &self,
        module: &crate::ir::module::IrModule,
        source_manager: Option<&SourceManager>,
        input_file: &str,
        fp_environment_observable: bool,
        ra_config: std::sync::Arc<crate::backend::regalloc::RaConfig>,
        tune: crate::backend::x86::cpu_model::X86Tune,
        time_phases: bool,
        t0: std::time::Instant,
    ) -> Result<String, String> {
        if let Some(diagnostic) = unsettled_x86_requirement(module) {
            return Err(format!("{input_file}: {diagnostic}"));
        }
        // Generate assembly using target-specific codegen
        let t8 = std::time::Instant::now();
        let opts = crate::backend::CodegenOptions {
            ra_config,
            // At -O0 phi elimination leaves non-SSA multi-def webs. The linear
            // scan assumes one definition and miscompiled 125/200 CFG seeds;
            // use canonical stack homes until an SSA-aware O0 allocator exists.
            disable_regalloc: self.opt_level == 0,
            pic: self.pic || self.shared_lib,
            shared_lib: self.shared_lib,
            pie: self.pie && !self.pic && !self.shared_lib,
            no_plt: self.no_plt,
            // Only a FULL invocation that final-links in position-fixed
            // ET_EXEC may emit the absolute switch-table form (its jump
            // instruction's R_X86_64_32S is unresolvable in ET_DYN). The
            // PIE decision must match the linker's exactly — same
            // last-`-pie`/`-no-pie`-wins scan over the same argument list
            // the link receives — so the codegen and link never disagree.
            fixed_exec_link: matches!(self.mode, CompileMode::Full)
                && !self.shared_lib
                && !self.relocatable
                && !crate::backend::common::linker_pie_effective(&self.build_linker_args()),
            tls_model_floor: self.tls_model_floor,
            function_return_thunk: self.function_return_thunk,
            indirect_branch_thunk: self.indirect_branch_thunk,
            indirect_branch_thunk_inline: self.indirect_branch_thunk_inline,
            patchable_function_entry: self.patchable_function_entry,
            mcount: if self.mcount_pg {
                Some(self.mcount_submode)
            } else {
                None
            },
            cf_protection_branch: self.cf_protection_branch,
            stack_clash_protection: self.stack_clash_protection,
            stack_protector_guard: (self.stack_protector_mode
                != crate::backend::stack_protector::StackProtectorMode::None)
                .then(|| self.stack_protector_guard.resolve(self.target)),
            // GCC/Clang align function entries to 16 bytes at -O1..-O3 and
            // leave them unaligned at -Os/-Oz. Emitted as a real `.p2align` so
            // section alignment keeps following GNU as semantics.
            // `-falign-functions[=N]`/`-fno-align-functions` override: a
            // custom alignment must be a power of two (validated at parse).
            function_alignment: match self.align_functions {
                crate::passes::loop_align::AlignControl::Off => 0,
                crate::passes::loop_align::AlignControl::Custom { align, .. } => align,
                crate::passes::loop_align::AlignControl::Auto => {
                    if self.opt_level >= 1 && self.opt_level <= 3 {
                        16
                    } else {
                        0
                    }
                }
            },
            skip_rax_setup: self.skip_rax_setup,
            no_sse: self.codegen_no_sse(),
            i686_no_sse2: self.target == Target::I686
                && (self.general_regs_only || !self.resolved_i686_sse2()),
            i686_no_sse3: self.target == Target::I686
                && (self.general_regs_only || !self.resolved_i686_sse3()),
            general_regs_only: self.general_regs_only,
            code_model_kernel: self.code_model_kernel,
            no_jump_tables: self.no_jump_tables,
            bmi1: self.resolved_codegen_bmi1(),
            bmi2: self.resolved_bmi2(),
            apx: self.enable_apxf,
            tune,
            lzcnt: self.resolved_lzcnt(),
            popcnt: self.resolved_popcnt(),
            no_prefetch: std::env::var_os("CCC_NO_PREFETCH").is_some(),
            x86_prefetch: crate::backend::x86_common::X86PrefetchIsa {
                sse: self.resolved_prefetch_sse(),
                prfchw: self.resolved_prfchw(),
            },
            avx512: self.enable_avx512f,
            avx512vl: self.enable_avx512f && self.enable_avx512vl,
            isa: self.x86_isa(),
            no_relax: self.riscv_no_relax,
            debug_info: self.debug_info,
            function_sections: self.function_sections,
            data_sections: self.data_sections,
            code16gcc: self.code16gcc,
            regparm: self.regparm,
            fp_contract: if fp_environment_observable {
                crate::common::fp_contract::FpContract::Off
            } else {
                self.resolved_fp_contract()
            },
            fma: self.enable_fma,
            preferred_stack_bytes: self.preferred_stack_bytes,
            omit_frame_pointer: self.omit_frame_pointer,
            emit_cfi: !self.no_unwind_tables,
            // opt_level 0..=3 = speed/size-balanced or speed; 4 (-Os) / 5 (-Oz)
            // = optimize for size. Codegen uses this to pick the shorter
            // sequence (idiv/imul) over the faster one (magic mul, LEA chains).
            optimize_for_size: self.opt_level >= 4,
            zero_call_used_regs: self.zero_call_used_regs,
            zero_call_used_regs_fp_simd: self.resolved_zero_call_used_regs_fp_simd(),
        };
        let asm = self
            .target
            .generate_assembly_with_opts_and_debug(module, &opts, source_manager);
        self.validate_target_assembly(module, &asm, input_file)?;
        if time_phases {
            eprintln!(
                "[TIME] codegen: {:.3}s ({} bytes asm)",
                t8.elapsed().as_secs_f64(),
                asm.len()
            );
        }

        if time_phases {
            eprintln!(
                "[TIME] total compile {}: {:.3}s",
                input_file,
                t0.elapsed().as_secs_f64()
            );
        }

        if self.verbose {
            eprintln!("Generated {:?} assembly ({} bytes)", self.target, asm.len());
        }

        // Runtime CPU probe helper (AUD-CPU-1): appended only when a probe
        // site was actually lowered, so a translation unit that never asks
        // about CPU features carries no extra bytes.  The helper's symbols are
        // `.local`: several objects in one link each get their own copy, none
        // of them can collide, and no user definition can preempt the
        // initialiser the generated call sites resolve to.
        let probe_target = match self.target {
            Target::X86_64 => Some(crate::common::cpu_probe::ProbeTarget::X86_64),
            Target::I686 => Some(crate::common::cpu_probe::ProbeTarget::I686),
            Target::Aarch64 | Target::Riscv64 => None,
        };
        let asm = match probe_target {
            Some(probe_target) if crate::common::cpu_probe::take_runtime_needed() => {
                let mut text = asm;
                text.push('\n');
                text.push_str(&crate::common::cpu_probe::runtime_asm(
                    probe_target,
                    self.pic || self.shared_lib || self.pie,
                    !self.no_unwind_tables,
                ));
                text
            }
            _ => asm,
        };

        Ok(asm)
    }
}

impl Default for Driver {
    fn default() -> Self {
        Self::new()
    }
}

impl Driver {
    /// x86-64 code-generation ISA permission for this translation unit.
    ///
    /// Policy (`backend::x86::isa`):
    /// * no explicit `-march`: the x86-64 baseline (SSE2 only); post-baseline
    ///   classes come from explicit `-m*` flags only;
    /// * explicit `-march=<level|cpu|native>`: the requested feature set is
    ///   the ceiling (GCC-exact), explicit denials still apply;
    /// * `-mno-sse` / `-mgeneral-regs-only`: nothing (the register file is
    ///   off-limits; the kernel runs with CR4.OSFXSR/OSXSAVE clear).
    ///
    /// The INTEGER half of the ISA, mirroring the SIMD ceiling in `x86_isa()`.
    /// With no explicit `-march=` the baseline is x86-64 and none of
    /// BMI1/BMI2/LZCNT/MOVBE/POPCNT is granted; each is granted only by its
    /// explicit flag or a `-march=` profile that includes it, and an explicit
    /// `-mno-*` denial always wins.  BMI/LZCNT/POPCNT/MOVBE are integer ISA:
    /// they stay legal under `-mno-sse` (same contract as
    /// `enable_x86_v3_profile`).
    ///
    /// GCC-exactness of the explicit-flag interaction, measured on the
    /// host oracle (gcc 14.2, `-dM -E` macro census — the Review-AI claim
    /// that "GCC is strictly last-wins and re-enables BMI1 under
    /// `-mno-bmi -march=x86-64-v3`" is FALSE):
    ///
    /// | flags | GCC `__BMI__` | LCCC `resolved_bmi1` |
    /// |---|---|---|
    /// | `-mno-bmi -march=x86-64-v3` | **absent** (explicit denial is sticky against the arch set) | false |
    /// | `-march=x86-64-v3 -mno-bmi` | absent (order-independent) | false |
    /// | `-mbmi -march=x86-64` | **defined** (explicit enable survives a lower arch) | true |
    /// | `-mno-bmi -mbmi` | defined (later explicit wins) | true |
    /// | `-march=x86-64-v3 -march=x86-64` | absent (later `-march=` REPLACES the set) | false (post-fix) |
    ///
    /// i.e. the model is "explicit mask overrides the final arch, in both
    /// directions", not plain last-wins.  A second `-march=` REPLACES the
    /// arch-implied set (the `-march=` arm resets the enable surface and
    /// replays the recorded explicit flags after applying the profile —
    /// see `reset_x86_arch_implied_isa` / `replay_x86_explicit_isa_flags`).
    pub(super) fn resolved_bmi1(&self) -> bool {
        self.target == Target::X86_64 && !self.bmi_explicitly_disabled && self.enable_bmi
    }

    /// BMI1 permission for scalar CODEGEN (`CodegenOptions::bmi1`): the
    /// `resolved_bmi1` grant, extended to i686, whose backend consumes it
    /// only for TZCNT (Ctz). GCC emits `tzcnt` for `-m32 -mbmi` and BSF for
    /// `-m32 -mlzcnt`; gating i686 TZCNT on LZCNT emitted it for LZCNT-only
    /// CPUs (K10, Bobcat, Jaguar), where F3 0F BC is BSF and Ctz(0) is
    /// undefined. `__BMI__` and the ANDN-dependent middle-end choices stay
    /// on the X86_64-locked `resolved_bmi1`: i686 lowers no ANDN/BLSR.
    pub(super) fn resolved_codegen_bmi1(&self) -> bool {
        matches!(self.target, Target::X86_64 | Target::I686)
            && !self.bmi_explicitly_disabled
            && self.enable_bmi
    }

    pub(super) fn resolved_bmi2(&self) -> bool {
        self.target == Target::X86_64 && !self.bmi2_explicitly_disabled && self.enable_bmi2
    }

    /// Intrinsic/header permission is distinct from BMI2 instruction selection:
    /// i686 supports the 32-bit BZHI builtin through a defined lowering, while
    /// its optimizer must not receive the x86-64-only BMI2 codegen grant.
    pub(super) fn resolved_builtin_bmi2(&self) -> bool {
        matches!(self.target, Target::X86_64 | Target::I686)
            && !self.bmi2_explicitly_disabled
            && self.enable_bmi2
    }

    /// LZCNT/ABM permission (Clz only; TZCNT is BMI1, see
    /// `resolved_codegen_bmi1`). Granted on x86-64 and i686 only by
    /// `-mlzcnt` or an ABM-implying `-march=`, always under the
    /// `-mno-lzcnt` veto -- the same explicit-only contract as GCC.
    ///
    /// Why it must never be granted speculatively: the F3 0F BD encoding
    /// does not fault on pre-ABM CPUs, it executes as BSR (the prefix is
    /// ignored) and silently returns the MSB *index* instead of the
    /// leading-zero *count*. Without the grant both backends lower Clz to
    /// BSR with an explicit zero fixup, keeping the IR's Clz(0) == width.
    pub(super) fn resolved_lzcnt(&self) -> bool {
        if self.lzcnt_explicitly_disabled {
            return false;
        }
        // GCC: ABM brings LZCNT unless an LZCNT option decided it; a
        // denial returned above, and `-mlzcnt` sets `enable_lzcnt`.
        match self.target {
            Target::X86_64 | Target::I686 => self.enable_lzcnt || self.enable_abm,
            _ => false,
        }
    }

    /// GCC's ABM state (`__ABM__`), on x86-64 and i686.
    pub(super) fn resolved_abm(&self) -> bool {
        matches!(self.target, Target::X86_64 | Target::I686) && self.enable_abm
    }

    /// Soundness: the instruction IS the builtin's semantics — the choice
    /// is only which lowering of `__builtin_popcount` wins (`popcnt` vs.
    /// the bit-count idiom), never a defined program's results.
    ///
    /// i686 parity (ISA-parity audit): the i686 backend has a wired
    /// `popcnt_enabled` consumer (i686/codegen/alu.rs) and the integrated
    /// assembler owns `popcntl` (`F3 0F B8`). Explicit-flag-only, exactly
    /// like [`Self::resolved_lzcnt`]: `popcntl` is #UD on pre-SSE4.2
    /// silicon, and the i686 baseline promises compatibility with that
    /// silicon — so the capability arm arms only under `-mpopcnt`/an
    /// ISA-implying `-march=`, under the `-mno-popcnt` veto. There is no
    /// i686 default grant even for these legacy-encoding forms.
    pub(super) fn resolved_popcnt(&self) -> bool {
        if self.popcnt_explicitly_disabled {
            return false;
        }
        match self.target {
            Target::X86_64 => self.enable_popcnt,
            Target::I686 => self.enable_popcnt,
            _ => false,
        }
    }

    /// PREFETCHW (`0F 0D /1`) permission: explicit `-mprfchw` or a
    /// PRFCHW-implying `-march=`, under the sticky `-mno-prfchw` veto. Never
    /// a default grant: the absent-march x86-64-v3 baseline does not include
    /// it (GCC 16: `__PRFCHW__` is undefined for `-march=x86-64-v3`, and
    /// Haswell — the v3 reference core — lacks it), and on a pre-Broadwell
    /// Intel core the opcode is not part of any architectural extension.
    pub(crate) fn resolved_prfchw(&self) -> bool {
        matches!(self.target, Target::X86_64 | Target::I686)
            && self.enable_prfchw
            && !self.prfchw_explicitly_disabled
    }

    /// `CodegenOptions::no_sse`: the backend may not use SSE moves. Legacy
    /// bulk-memory/vararg helpers use SSE2 integer moves, so an SSE2 denial
    /// turns them off too, independently of the SSE1 scalar operations the
    /// driver flags permit. The middle end reads the same answer
    /// (`loop_idiom::Capabilities::i686_sse`) wherever its decisions must
    /// match the emitter's.
    pub(crate) fn codegen_no_sse(&self) -> bool {
        self.no_sse || self.no_sse2 || (self.target == Target::I686 && !self.resolved_i686_sse())
    }

    /// Whether this i686 invocation enables the SSE register file.
    ///
    /// The i386 DEFAULT baseline is the x86-64 ISA level (SSE/SSE2 on):
    /// GCC 16.2's default `-m32` resolves to `-march=x86-64` with
    /// `-msse`/`-msse2` [enabled] (measured; Clang 23.1.3's default `-m32`
    /// agrees — 16-byte vectors take `%xmm0` in both), and lccc's i386
    /// preprocessor defines `__SSE__`/`__SSE2__` on the baseline too. The
    /// grant is vetoed only by an explicit pre-SSE `-march=` (i386/i486/
    /// i586/i686/pentium/pentiumpro/pentium-mmx) or the sticky `-mno-sse`;
    /// an explicit `-msse` (or any higher family grant) overrides the
    /// `-march=` veto, in either flag order (the replay machinery settles
    /// the order). The separate `general_regs_only` permission veto is
    /// applied by consumers such as `effective_x86_features`; prefetch has
    /// its own established `-mno-sse` policy below.
    pub(crate) fn resolved_i686_sse(&self) -> bool {
        self.target == Target::I686
            && !self.no_sse
            && (self.enable_sse
                || self.enable_sse3
                || self.enable_ssse3
                || self.enable_sse4_1
                || self.enable_sse4_2
                || self.enable_avx
                || self.enable_avx2
                || self.enable_avx512f
                || !self.march_pre_sse)
    }

    /// GCC's CPU-name macros, as measured per CPU in `arch_table.rs`:
    /// `__haswell`, `__haswell__`, `__core_avx2__`, ... for the `-march=`
    /// CPU (`x86-64` by default: `__k8`), `__tune_haswell__`, ... for the
    /// `-mtune=` CPU, or without one for the `-march=` CPU unless it is an
    /// `x86-64*` level (generic tuning). `gen_x86_arch_table.py --check`
    /// verifies both defaults against GCC for every CPU. The
    /// `__athlon_sse__`-style companions need SSE, as in GCC. `native`,
    /// `generic` and names invalid for the target define nothing.
    pub(crate) fn x86_cpu_name_macros(&self) -> Vec<&'static str> {
        use crate::backend::x86::isa::X86Isa;
        if !matches!(self.target, Target::X86_64 | Target::I686) {
            return Vec::new();
        }
        let i686 = self.target == Target::I686;
        let sse = if i686 {
            self.resolved_i686_sse()
        } else {
            !self.no_sse
        };
        let arch = X86Isa::arch_row(self.x86_march.as_deref().unwrap_or("x86-64"));
        let tune = match self.x86_tune.as_deref() {
            Some(tune) => X86Isa::arch_row(tune),
            None => self
                .x86_march
                .as_deref()
                .filter(|arch| !arch.starts_with("x86-64"))
                .and_then(X86Isa::arch_row),
        };
        let arch = arch
            .filter(|row| row.x86_64 || i686)
            .map_or(&[][..], |row| row.arch_macros);
        let tune = tune
            .filter(|row| row.x86_64 || i686)
            .map_or(&[][..], |row| row.tune_macros);
        arch.iter()
            .chain(tune)
            .copied()
            .filter(|name| sse || !name.ends_with("_sse__"))
            .collect()
    }

    /// Whether this i686 invocation defines `__MMX__`. MMX follows the arch
    /// (any SSE CPU, or a pre-SSE CPU with MMX) or an SSE grant, unless a
    /// full register-file denial is active: `-m32 -mgeneral-regs-only`
    /// defines none of __SSE__/__SSE2__/__MMX__ (measured), unlike
    /// `-mno-sse`, which keeps __MMX__ (both oracles). x86-64 keeps __MMX__
    /// under `-mgeneral-regs-only`; the i386 asymmetry is GCC's own
    /// (measured on 16.2).
    pub(crate) fn resolved_i686_mmx(&self) -> bool {
        (!self.march_pre_sse || self.march_pre_sse_mmx || self.resolved_i686_sse())
            && !self.general_regs_only
            && !self.mmx_explicitly_disabled
    }

    /// Whether this i686 invocation enables the SSE2 level (above the SSE
    /// level of [`Self::resolved_i686_sse`]). The baseline and vetoes are
    /// identical; additionally the sticky `-mno-sse2` denies only this
    /// level — GCC 16.2 measures `-m32 -mno-sse2` as `-msse` [enabled] /
    /// `-msse2` [disabled] — and every SSE3-and-up grant implies it (as
    /// does AVX), exactly like the explicit `-msse2` bit.
    pub(crate) fn resolved_i686_sse2(&self) -> bool {
        self.target == Target::I686
            && !self.no_sse
            && !self.sse2_explicitly_disabled
            && (self.enable_sse2
                || self.enable_sse3
                || self.enable_ssse3
                || self.enable_sse4_1
                || self.enable_sse4_2
                || self.enable_avx
                || self.enable_avx2
                || self.enable_avx512f
                || !self.march_pre_sse)
    }

    /// Whether this i686 invocation enables the SSE3 level (FISTTP and the
    /// SSE3 vector forms). SSE3 needs the SSE2 level, and every grant at or
    /// above it implies it; the denials (`-mno-sse3` and up, `-mno-sse2`,
    /// `-mno-sse`) clear the grant bits themselves. The default `-m32`
    /// baseline (`-march=x86-64`, GCC 16.2) has no SSE3, so neither does
    /// lccc's: FISTTP there is a #UD on SSE2-only silicon (Pentium 4
    /// Willamette/Northwood, Athlon 64 before revision E).
    pub(crate) fn resolved_i686_sse3(&self) -> bool {
        self.resolved_i686_sse2()
            && (self.enable_sse3
                || self.enable_ssse3
                || self.enable_sse4_1
                || self.enable_sse4_2
                || self.enable_avx
                || self.enable_avx2
                || self.enable_avx512f)
    }

    /// Whether the SSE prefetch family (`prefetchnta/t0/t1/t2`, `0F 18
    /// /0-3`) may be emitted — GCC's `ix86_prefetch_sse`:
    ///
    /// * x86-64: always. The forms are part of the x86-64 baseline and touch
    ///   no vector state, so GCC 16 emits them even under `-mno-sse` and
    ///   `-mgeneral-regs-only` (measured).
    /// * i686: SSE enabled, or PRFCHW enabled (GCC: `TARGET_SSE ||
    ///   (TARGET_PRFCHW && !TARGET_3DNOW)`; `-m3dnow` is rejected here, so
    ///   the 3DNow! clause is vacuous). The i686 default is pre-SSE silicon,
    ///   where these opcodes are #UD, and GCC emits nothing for
    ///   `-m32`/`-march=i686`/`-mno-sse` (measured).
    pub(crate) fn resolved_prefetch_sse(&self) -> bool {
        match self.target {
            Target::X86_64 => true,
            Target::I686 => self.resolved_i686_sse() || self.resolved_prfchw(),
            _ => false,
        }
    }

    /// Whether the target has a direct hardware population-count lowering
    /// (`popcnt` on x86/i686, AdvSIMD `cnt` + `addv` on AArch64 when SIMD
    /// registers are permitted). Used by `loop_idiom` so `while (x) { x &= x - 1; n++; }`
    /// folds to `IrUnaryOp::Popcount` only when the backend lowers `Popcount`
    /// without a software bit-count loop (GCC 16 / Clang 23 / ICX 2026 parity).
    pub(crate) fn resolved_fast_popcount(&self) -> bool {
        match self.target {
            Target::X86_64 | Target::I686 => self.resolved_popcnt(),
            Target::Aarch64 => !self.general_regs_only,
            Target::Riscv64 => false,
        }
    }

    /// Effective FP contraction policy (GCC parity, measured 2026-10-13 on
    /// gcc 14.2 with `-march=x86-64-v3` probes).
    ///
    /// GCC never fuses `a*b + c` into an FMA at `-O0` or `-O1` — not even when
    /// the target has FMA and not even under `-ffast-math`; the fusion appears
    /// at `-O2`.  LCCC's C default is `-ffp-contract=fast` (which matches
    /// GCC's *flag* default), and applying it at every optimization level made
    /// `-O0`/`-O1` output differ from GCC's in the last ulp(s) — which is how
    /// 26 in-repo reproducers came to be filed as "miscompile" although both
    /// results are conforming.  An explicit `-ffp-contract=` or `-ffast-math`
    /// still wins at any level.
    pub(super) fn resolved_fp_contract(&self) -> crate::common::fp_contract::FpContract {
        use crate::common::fp_contract::FpContract;
        if !self.fp_contract_explicit && self.opt_level < 2 {
            return FpContract::Off;
        }
        self.fp_contract
    }

    pub(super) fn resolved_cx16(&self) -> bool {
        self.target == Target::X86_64 && !self.cx16_explicitly_disabled && self.enable_cx16
    }

    pub(super) fn resolved_movbe(&self) -> bool {
        self.target == Target::X86_64 && !self.movbe_explicitly_disabled && self.enable_movbe
    }

    /// MOVBE on the target CPU, x86-64 or i686: GCC's `__MOVBE__` and the
    /// compile-time feature set. Code generation reads `resolved_movbe`
    /// (the i686 backend selects no MOVBE), and no lccc header keys an
    /// intrinsic on the macro, so announcing it on i686 promises nothing
    /// the compiler cannot compile.
    pub(super) fn resolved_isa_movbe(&self) -> bool {
        matches!(self.target, Target::X86_64 | Target::I686)
            && !self.movbe_explicitly_disabled
            && self.enable_movbe
    }

    /// Effective x86 feature set — the compile-time guarantees this invocation
    /// gives the generated code (AUD-CPU-1).
    ///
    /// This is the *codegen permission* model, not the "what did the command
    /// line ask for" model the predefined macros (`__AVX2__`, `__BMI__`) use:
    ///
    /// * features the resolved ISA ceiling grants by default (project baseline
    ///   x86-64 when no `-march=` is given) are present, exactly like
    ///   [`Self::x86_isa`]'s default;
    /// * an explicit `-march=<level|cpu>` replaces the ceiling, so a v2 target
    ///   reports no AVX2;
    /// * `-mno-<feat>` denials are honoured through the same resolved
    ///   predicates the backend consults (`resolved_bmi1`, `x86_isa`, ...);
    /// * non-x86 targets get an empty set — `__builtin_cpu_supports` is not
    ///   available there at all (see the lowering's diagnostic).
    ///
    /// Kept next to `x86_isa` on purpose: whoever changes the ceiling must see
    /// both consumers in one screen, so the folded builtin and the emitted
    /// instructions cannot disagree.
    pub(crate) fn effective_x86_features(&self) -> crate::common::x86_features::EffectiveFeatures {
        use crate::common::x86_features::{EffectiveFeatures, X86Feature as F};
        let mut set = EffectiveFeatures::EMPTY;
        if !matches!(self.target, Target::X86_64 | Target::I686) {
            return set;
        }
        let x86_64 = self.target == Target::X86_64;
        // xmm-level code is legal at all (mirrors `x86_isa()` / `set_sse_macros`).
        let simd = !self.no_sse && !self.general_regs_only;

        // Scalar features every supported x86 CPU has.
        set.insert_if(x86_64, F::Cmov);
        set.insert_if(x86_64, F::Fxsr);
        set.insert_if(x86_64 && simd, F::Mmx);

        // SSE/SSE2 are part of the x86-64 architectural baseline (v1 already
        // has them). i686's default baseline is the same ISA level (GCC 16.2
        // default `-m32` = `-march=x86-64`, measured) — publish its resolved
        // SSE/SSE2 grants so ABI classification (the 16-byte XMM vector
        // carrier, `__SSE__`/`__SSE2__` and friends) tracks the codegen:
        // only an explicit pre-SSE `-march=` / `-mno-sse` / `-mno-sse2`
        // drops to the pre-SSE stack/sret convention.
        //
        // The SSE2 bit rides the order-sensitive `no_sse2` denial, NOT the
        // SSE1 level (mirrors `x86_isa()`, whose NONE arm covers
        // `no_sse || no_sse2 || general_regs_only`): an explicit `-mno-sse2`
        // keeps `F::Sse` (scalar SSE1 and the typed SSE1 F32 maps stay
        // legal) while dropping `F::Sse2`, and a later `-msse` must not
        // reopen it — the `-msse` handler clears `no_sse2` only when no
        // explicit SSE2 denial is pending. Publishing Sse2 under an SSE1-only
        // target let the `__builtin_shufflevector` i32x4 lowering take the
        // `pshufd` path and emit an illegal instruction (or hit the
        // fail-closed SSE2 scan as a hard error); with the bit dropped the
        // lowering keeps its portable scalar gather, exactly what GCC 16.2
        // does for `-mno-sse2 -msse` (measured: scalar oracle, no shuffles).
        set.insert_if(simd && (x86_64 || self.resolved_i686_sse()), F::Sse);
        set.insert_if(
            simd && !self.no_sse2 && (x86_64 || self.resolved_i686_sse2()),
            F::Sse2,
        );

        // The SSE3..SSE4.2 ladder.  `x86_isa()` carries only the 128-bit
        // SSE4.1 class, so SSE3/SSSE3/SSE4.2 have no codegen bit of their
        // own. Each rung is granted only by its explicit flag (or a `-march`
        // profile that sets it): the default x86-64 baseline has none of them.
        set.insert_if(simd && self.enable_sse3, F::Sse3);
        set.insert_if(simd && self.enable_ssse3, F::Ssse3);
        // The x86-64 codegen ceiling is deliberately NONE for i686, but
        // i686 still has explicit 128-bit SSE4.1 intrinsic emitters. Publish
        // that request from its real SSE2 prerequisite, not the other backend.
        let legacy_sse41 =
            set.contains(F::Sse2) && self.enable_sse4_1 && !self.sse41_explicitly_disabled;
        set.insert_if(legacy_sse41, F::Sse41);
        set.insert_if(simd && self.enable_sse4_2, F::Sse42);
        set.insert_if(self.resolved_popcnt(), F::Popcnt);
        // CMPXCHG16B: granted only by `-mcx16` or a v2+ `-march` profile.
        set.insert_if(self.resolved_cx16(), F::Cx16);

        // SIMD classes straight from the codegen ceiling.
        let isa = self.x86_isa();
        // The i686 native intrinsic emitters support VEX even though the
        // x86-64-only vectorizer ceiling above deliberately remains NONE.
        // Do not borrow that backend policy to reject explicitly requested
        // i686 instructions, or widen the vectorizer ceiling to admit them.
        let i686_avx =
            !x86_64 && set.contains(F::Sse2) && self.enable_avx && !self.avx_explicitly_disabled;
        set.insert_if(isa.avx || i686_avx, F::Avx);
        set.insert_if(
            isa.ymm || (i686_avx && self.enable_avx2 && !self.avx2_explicitly_disabled),
            F::Avx2,
        );
        set.insert_if(isa.fma, F::Fma);
        // F16C is VEX-encoded: `-mno-avx` removes it even when a `-march=`
        // profile asked for it (GCC undefines `__F16C__` for the same
        // combination — measured). It is granted only by `-mf16c` or the
        // `-march` profile that includes it, never by the default baseline.
        set.insert_if(simd && isa.avx && self.enable_f16c, F::F16c);
        // AES-NI and PCLMUL are NOT part of any psABI microarchitecture level
        // (v2 has SSE4.2/POPCNT, v3 adds AVX2/BMI2/FMA/F16C). Granting them
        // from a default baseline would promise an instruction class the target
        // level does not imply, so they follow the explicit request only —
        // and nothing in the backend emits them without `-maes`/`-mpclmul`
        // (or an `-march=<cpu>` profile), which is what makes the claim true.
        // Both have legacy (non-VEX) encodings, so `-mno-avx` does not
        // remove them.
        set.insert_if(simd && self.enable_aes, F::Aes);
        set.insert_if(simd && self.enable_pclmul, F::Pclmul);
        // VAES / VPCLMULQDQ: the EVEX 256-bit forms of `aesenc` and
        // `pclmulqdq`.  Two conditions, both measured against GCC 12.2:
        //   * the 256-bit vector class must be permitted (`isa.ymm`, the
        //     AVX2-and-not-denied ceiling).  `-mvaes` alone rejects
        //     `_mm256_aesenc_epi128`; `-mavx2 -mvaes` and `-mavx512f -mvaes`
        //     accept it -- AVX-512 brings its own 256-bit permission, which is
        //     why this follows `isa` (and not a second `-mavx2` test) rather
        //     than the raw `enable_avx2` flag;
        //   * the extension itself must be requested, exactly like AES-NI:
        //     no `-march=` profile in the table grants it implicitly, and the
        //     backend emits the 256-bit classes only for the `-mvaes` /
        //     `-mvpclmulqdq` request.
        // Folding `1` here promises the class, and the two halves of the
        // conjunction are the two ways the promise could be wrong: without
        // `isa.avx` the emitter cannot encode the instruction, and without the
        // request flag the backend was never asked for it.
        // VAES/VPCLMULQDQ at 256 bits are VEX.256 forms that need AVX only, not
        // AVX2 (measured: gcc 14 accepts `-mvaes -mno-avx2` and `-mvaes` alone;
        // rejects `-mvaes -mno-avx`). So the class conjunct is `avx`, not `ymm`.
        set.insert_if(isa.avx && self.enable_vaes, F::Vaes);
        set.insert_if(isa.avx && self.enable_vpclmulqdq, F::Vpclmulqdq);

        // The three AVX-VNNI groups: `vpdpbusd`/`vpdpwssd` (AVX-VNNI),
        // `vpdpbssd`/`vpdpbsud`/`vpdpbuud` (AVX-VNNI-INT8) and
        // `vpdpwusd`/`vpdpwuud` (AVX-VNNI-INT16).  Distinct CPUID bits
        // (leaf 7: EBX bit 4, ECX bit 4, EDX bit 4) and distinct GCC flags,
        // so they get distinct bits here -- one bit each, never a shared
        // "vnni" umbrella.
        //
        // Shape, measured against GCC 12.2 (`-march=raptorlake` is unknown to
        // it; `-march=alderlake` is the CPU profile):
        //
        //   _mm_dpbusd_epi32 / _mm256_dpbusd_epi32
        //     -march=x86-64               reject   <- lccc emitted vpdpbusd
        //     -march=x86-64-v3            reject      in ALL of these columns,
        //     -mavxvnni                   accept      i.e. an AVX-VNNI
        //     -mavxvnni -mno-avx          reject      instruction inside a v1
        //     -mavxvnni -mno-avx2         reject      object: the SIGILL class
        //     -mavx512vnni                reject      AUD-CPU-3 exists for
        //     -mavx512vnni -mavx512vl     accept
        //     -march=alderlake            accept
        //
        // The measured rows are the VEX-256 class (`isa.ymm`, which AVX-512
        // also grants) conjoined with the extension's own flag -- the same
        // shape as VAES/VPCLMULQDQ above, and the same one GCC's header guard
        // implements.  One row deviates and is asserted as such by
        // tests/regression/check_intrinsic_feature_gate.sh: GCC's `-mavxvnni`
        // also implies AVX2, so `-march=x86-64 -mavxvnni` is accepted there
        // while this fold refuses it, because the user asked for a v1 baseline
        // and the 256-bit class is what the backend actually needs to emit the
        // instruction.  Refusing a request is the safe direction; emitting an
        // unrequested instruction is not.
        //
        // GCC 12.2 cannot be an oracle for the INT8/INT16 halves: it does not
        // know either flag (measured: `unrecognized command-line option
        // '-mavxvnniint8'`), and its headers declare none of those intrinsics.
        // Those rows are therefore pinned by lccc's own flag contract, and the
        // gate says so instead of pretending to compare.
        //
        // `F::AvxVnni` is the CPUID AVX-VNNI bit (VEX), exactly GCC's
        // `__AVXVNNI__`; AVX512-VNNI+VL reaches the same intrinsics through
        // EVEX: AVX512-VNNI with AVX512-VL (`isa::function_evex_vnni_permitted`).
        set.insert_if(isa.ymm && self.enable_avxvnni, F::AvxVnni);
        set.insert_if(isa.ymm && self.enable_avxvnniint8, F::AvxVnniInt8);
        set.insert_if(isa.ymm && self.enable_avxvnniint16, F::AvxVnniInt16);

        // GFNI: `gf2p8mulb` and the two affine transforms.  Legacy-encoded for
        // xmm (there is no non-VEX-free requirement), so -- unlike the four
        // extensions above -- the permission is the vector class alone plus the
        // extension's own flag, and `-mno-avx` does NOT remove it.  Measured
        // against GCC 12.2: `-mgfni` accepts `_mm_gf2p8mul_epi8` with and
        // without `-mavx`, `-march=x86-64`/`-v3`/`-maes` reject it, and before
        // this row lccc emitted `gf2p8mulb %xmm1, %xmm0` for `-march=x86-64`:
        // the same SIGILL class as the VNNI case, on an instruction whose
        // *legacy* encoding makes `-mno-avx` useless as a defence.
        set.insert_if(simd && self.enable_gfni, F::Gfni);

        // AVX-512 helpers are instruction-level capabilities, not a blanket
        // consequence of the 512-bit result carrier. Publish only explicitly
        // granted subsets, and require the F base for every subset bit. `-mavx512bw`
        // etc. imply F in argument processing, but retaining the conjunction
        // here keeps a later `-mno-avx512f` denial fail closed.
        let avx512_base = simd && self.enable_avx512f;
        set.insert_if(avx512_base, F::Avx512F);
        set.insert_if(avx512_base && self.enable_avx512bw, F::Avx512Bw);
        set.insert_if(avx512_base && self.enable_avx512dq, F::Avx512Dq);
        set.insert_if(avx512_base && self.enable_avx512vl, F::Avx512Vl);
        set.insert_if(avx512_base && self.enable_avx512vnni, F::Avx512Vnni);
        set.insert_if(avx512_base && self.enable_avx512bitalg, F::Avx512BitAlg);
        set.insert_if(
            avx512_base && self.enable_avx512vpopcntdq,
            F::Avx512Vpopcntdq,
        );
        set.insert_if(avx512_base && self.enable_avx512cd, F::Avx512Cd);
        set.insert_if(avx512_base && self.enable_avx512ifma, F::Avx512Ifma);
        set.insert_if(avx512_base && self.enable_avx512vbmi, F::Avx512Vbmi);
        set.insert_if(avx512_base && self.enable_avx512vbmi2, F::Avx512Vbmi2);
        set.insert_if(avx512_base && self.enable_avx512bf16, F::Avx512Bf16);
        set.insert_if(avx512_base && self.enable_avx512fp16, F::Avx512Fp16);
        set.insert_if(
            avx512_base && self.enable_avx512vp2intersect,
            F::Avx512Vp2intersect,
        );
        set.insert_if(avx512_base && self.enable_avx512er, F::Avx512Er);
        set.insert_if(avx512_base && self.enable_avx512pf, F::Avx512Pf);
        set.insert_if(avx512_base && self.enable_avx512bmm, F::Avx512Bmm);

        // Integer ISA: the resolved predicates already encode the default
        // grant, the `-march` ceiling and the sticky denials — the same
        // predicates `__BMI__`/`__BMI2__`/`__LZCNT__`/`__MOVBE__` use.
        set.insert_if(self.resolved_codegen_bmi1(), F::Bmi);
        set.insert_if(self.resolved_builtin_bmi2(), F::Bmi2);
        set.insert_if(self.resolved_lzcnt(), F::Lzcnt);
        set.insert_if(self.resolved_isa_movbe(), F::Movbe);
        // RDRAND has no resolved predicate and no explicit-denial bit; the
        // request flag is the only signal, so it is used as-is (an
        // unrequested feature folds to 0, the safe direction).
        set.insert_if(x86_64 && self.enable_rdrnd, F::Rdrnd);
        set
    }

    /// Code-generation ISA permission for the x86-64 SIMD world.
    /// Non-x86-64 targets get [`X86Isa::NONE`]; i686 keeps its own SSE2
    /// handling through `no_sse` and never reaches the x86-64 emitters.
    pub(crate) fn x86_isa(&self) -> crate::backend::x86::isa::X86Isa {
        use crate::backend::x86::isa::X86Isa;
        // The generic integer/F64 vector classes require SSE2. SSE1 F32 maps
        // have a separate typed permission: never reopen this wider class
        // merely because -msse follows an explicit SSE2 denial.
        if self.target != Target::X86_64 || self.no_sse || self.no_sse2 || self.general_regs_only {
            return X86Isa {
                cx16: self.resolved_cx16(),
                ..X86Isa::NONE
            };
        }
        // MOVBE is orthogonal to the SIMD ceiling: `resolved_movbe` owns its
        // permission (`-mno-movbe`, or an explicit `-march=` that does not
        // include it), so it is carried through both arms below.
        let movbe = self.resolved_movbe();
        let cx16 = self.resolved_cx16();
        // One rule for every spelling: a class is granted only by its explicit
        // flag (or a -march profile that sets it). The default x86-64 baseline
        // is therefore SSE2 only: no post-x86-64 instruction is ever emitted.
        let ceiling = {
            X86Isa {
                simd: true,
                sse41: self.enable_sse4_1,
                avx: self.enable_avx,
                ymm: self.enable_avx2,
                fma: self.enable_fma,
                movbe,
                cx16: false,
            }
        };
        X86Isa {
            simd: true,
            sse41: ceiling.sse41 && !self.sse41_explicitly_disabled,
            avx: ceiling.avx && !self.avx_explicitly_disabled,
            // The 256-bit class dies on EITHER an AVX denial (no VEX at
            // all) or an AVX2 denial (VEX.128 stays; `-mno-avx2` is the
            // AVX1+FMA target class, where GCC keeps every VEX.128 form
            // including the scalar FMA families).
            ymm: ceiling.ymm && !self.avx_explicitly_disabled && !self.avx2_explicitly_disabled,
            fma: ceiling.fma && !self.fma_explicitly_disabled,
            // NOT `&& !self.sse41_explicitly_disabled` and not gated on
            // `simd`: MOVBE has no SSE dependency, and `-mno-sse` must leave
            // it intact.
            movbe: ceiling.movbe,
            // CMPXCHG16B is integer ISA: `-mno-sse` does not remove it.
            cx16,
        }
        .normalized()
    }
}

/// First instruction line of `asm` that names an SSE/AVX register, with the
/// enclosing function's symbol.  Inline-asm bodies (`#APP` … `#NO_APP`) are
/// the user's own responsibility and are skipped; comments and directives
/// never count.
/// True when the final assembly text carries at least one DWARF debug
/// record.  lccc's debug emission is `.file`/`.loc` line-table directives
/// (GAS lowers them to `.debug_line`); either directive present means the
/// TU got some debug information, neither means `-g` was effectively
/// dropped and the honesty warning in the pipeline must fire.
fn asm_contains_debug_records(asm: &str) -> bool {
    asm.contains(".file ") || asm.contains(".loc ")
}

/// Check compiler-generated AArch64 instructions, not data, comments, symbol
/// names, or explicit inline assembly (the same policy as the x86 guard).
/// Emitted instructions occupy one line; no allocation is needed on success.
fn first_arm_simd_reference(asm: &str) -> Option<(String, String)> {
    let mut function = "<toplevel>";
    let mut in_app = false;
    for line in asm.lines() {
        let text = line.trim();
        if matches!(text, "#APP" | "//APP" | "// APP") {
            in_app = true;
            continue;
        }
        if matches!(text, "#NO_APP" | "//NO_APP" | "// NO_APP") {
            in_app = false;
            continue;
        }
        if in_app || text.is_empty() || text.starts_with('#') {
            continue;
        }
        if let Some(rest) = text.strip_prefix(".type ") {
            if rest.ends_with("function") {
                function = rest.split(',').next().unwrap_or("<toplevel>").trim();
            }
        }
        let text = text.split("//").next().unwrap_or("").trim();
        if text.starts_with('.') || text.ends_with(':') {
            continue;
        }
        let Some((mnemonic, operands)) = text.split_once(char::is_whitespace) else {
            continue;
        };
        // These operand slots contain labels which may legally be named v0,
        // d0, etc. They are addresses, not register-file references.
        if matches!(mnemonic, "b" | "bl" | "prfm" | "prfum") || mnemonic.starts_with("b.") {
            continue;
        }
        let operands = if matches!(mnemonic, "adr" | "adrp" | "cbz" | "cbnz" | "tbz" | "tbnz")
            || mnemonic.starts_with("ldr")
        {
            operands.split(',').next().unwrap_or("")
        } else {
            operands
        };
        for operand in operands.split(',') {
            let operand = operand.trim();
            if operand.starts_with(['#', '=', ':']) {
                continue;
            }
            for token in operand.split(|c: char| !c.is_ascii_alphanumeric() && c != '_') {
                let bytes = token.as_bytes();
                if bytes.len() >= 2
                    && matches!(bytes[0], b'v' | b'q' | b'd' | b's' | b'h' | b'b')
                    && bytes[1..].iter().all(u8::is_ascii_digit)
                {
                    return Some((function.to_owned(), line.to_owned()));
                }
            }
        }
    }
    None
}

impl Driver {
    /// Whether `-std=` selected a dialect before C99 (C90, or C94 =
    /// `iso9899:199409`): GCC's `flag_isoc99` is clear there, and sema
    /// follows it where the dialects differ.
    pub(crate) fn pre_c99_dialect(&self) -> bool {
        matches!(self.stdc_version, Some(None) | Some(Some("199409L")))
    }

    /// The warning config every diagnostic engine in this pipeline should
    /// run with: the user's `-Werror`/`-Wno-*` requests, plus GCC 16.2's
    /// measured dialect severity policy for the C99 default-error warnings
    /// (`-Wimplicit-int`, `-Wimplicit-function-declaration`,
    /// `-Wreturn-mismatch`, `-Wint-conversion`): from C99 on (and by
    /// default) they are errors, while pre-C99 dialects keep them as
    /// warnings. Explicit `-Wno-error=`/`-Wno-<flag>` escapes always win
    /// (`WarningConfig::apply_gcc162_c99_default_errors` skips them), so
    /// the promotion is dialect-driven, never unconditional.
    pub(crate) fn effective_warning_config(&self) -> crate::common::error::WarningConfig {
        let mut config = self.warning_config.clone();
        if self.pre_c99_dialect() {
            config.suppress_gcc162_pre_c99_defaults();
        } else {
            config.apply_gcc162_c99_default_errors();
        }
        config
    }

    /// Whether the active dialect is C23 or later (c23/gnu23/c2x/gnu2x/c2y/gnu2y).
    /// GCC 16.2's default is GNU C23 (`__STDC_VERSION__` 202311L); an explicit
    /// older `-std=` selects its own mode. Lexer keywords, `#if true`, `auto`
    /// type inference and the other C23-only meanings key off this one fact.
    pub(crate) fn c23_dialect(&self) -> bool {
        matches!(self.stdc_version, Some(Some(v))
            if v.trim_end_matches('L').parse::<u32>().is_ok_and(|n| n >= 202311))
    }
}
#[cfg(test)]
mod arm_simd_reference_tests {
    use super::first_arm_simd_reference;

    #[test]
    fn detects_scalar_vector_loads_stores_and_lane_operands() {
        for instruction in [
            "fadd d0, d1, d2",
            "fmov x0, d31",
            "cnt v0.8b, v1.8b",
            "ldr q0, [x0]",
            "str s0, [x1]",
            "stp q8, q9, [sp]",
            "ld1 {v0.4s}, [x0]",
            "umov w0, v31.s[0]",
        ] {
            let asm = format!(".type probe, %function\nprobe:\n    {instruction}\n");
            let (function, _) = first_arm_simd_reference(&asm).unwrap();
            assert_eq!(function, "probe");
        }
    }

    #[test]
    fn does_not_confuse_symbols_comments_immediates_or_user_assembly_with_registers() {
        let asm = ".type v0, %function\nv0:\nadr x0, d0\nadrp x1, :got:q0\n\
                   ldr x2, s0\nadd x0, x0, #:lo12:v0\nadd x1,x1,:lo12:v0\nmov x0, #d0\n\
                   b v0\nbl q31\nb.eq d0\ncbz x0,v0\ntbz x1,#1,q0\nprfm pldl1keep,v0\nmov x0, x1 // v1.8b\n\
                   .asciz \"v0 q0 d0\"\n#APP\nfmov d0, x0\n#NO_APP\nret\n";
        assert!(first_arm_simd_reference(asm).is_none());
    }
}

/// The first compiler-generated instruction a function's own
/// `target("...")` attribute forbids, as `(function, line, unit, entry)`,
/// where `entry` is the option that removed the unit: the SSE register
/// class (`general-regs-only`, `no-sse`, an `arch=` CPU without SSE), the
/// SSE2 vocabulary (SSE1 stays legal: GCC 16.2 keeps `mulss` under
/// `no-sse2`), on i686 SSE3, and the x87 stack (`general-regs-only`,
/// `no-80387`). The SSE levels are the function's features in GCC's
/// semantics (`isa::function_feature_bits`, from the TU's `tu` and
/// `tu_explicit`), the ones codegen reads, so a later grant
/// (`no-sse2,aes`) lifts a denial here as it does there. A unit the TU
/// already lacks is the TU-level scan's to report. Functions absent from
/// the final text are skipped.
/// The diagnostic for the first body that still holds an x86 requirement
/// an `always_inline` function deferred to its callers
/// ([`crate::ir::module::X86IsaNeeds`]). Inlining settles every requirement
/// its caller grants, so one that survives would be emitted where its
/// intrinsic is illegal; GCC refuses the same source ("inlining failed in
/// call to 'always_inline': target specific option mismatch"). What
/// survives is exactly what the function lacks, so all of it is named.
fn unsettled_x86_requirement(module: &crate::ir::module::IrModule) -> Option<String> {
    use crate::common::x86_features::X86Feature;
    let unsettled = module
        .functions
        .iter()
        .find(|f| !f.is_declaration && f.x86_isa_needs.deferred != 0)?;
    let calls_it = |f: &&crate::ir::module::IrFunction| {
        f.blocks.iter().flat_map(|b| &b.instructions).any(|inst| {
            matches!(inst, crate::ir::instruction::Instruction::Call { func, .. }
                if *func == unsettled.name)
        })
    };
    let site = module
        .functions
        .iter()
        .filter(|f| f.name != unsettled.name)
        .find(calls_it)
        .unwrap_or(unsettled);
    let needs = &unsettled.x86_isa_needs;
    let missing: Vec<X86Feature> = X86Feature::ALL
        .into_iter()
        .filter(|&f| needs.deferred & 1 << f as u8 != 0)
        .collect();
    let names: Vec<&str> = missing.iter().map(|f| f.name()).collect();
    let flags: Vec<String> = names.iter().map(|n| format!("-m{n}")).collect();
    Some(format!(
        "in function '{}': `{}` requires {} ({}), which the requested target does not \
         have; inlining failed in call to 'always_inline' '{}': target specific option \
         mismatch",
        site.name,
        needs.first_use.as_deref().unwrap_or(&unsettled.name),
        names.join(", "),
        flags.join(" "),
        unsettled.name,
    ))
}

fn first_target_attribute_leak(
    module: &crate::ir::module::IrModule,
    asm: &str,
    i686: bool,
    tu: u64,
    tu_explicit: u64,
) -> Option<(String, String, &'static str, String)> {
    use crate::common::x86_features::X86Feature as F;
    type Scan = fn(&str) -> Option<(String, String)>;
    fn sse3(body: &str) -> Option<(String, String)> {
        first_sse3_reference_unless(body, |_| false)
    }
    let levels: [(F, &'static str, Scan); 3] = [
        (F::Sse, "SSE", first_sse_reference),
        (F::Sse2, "SSE2", first_sse2_reference),
        (F::Sse3, "SSE3", sse3),
    ];
    for func in module
        .functions
        .iter()
        .filter(|f| !f.is_declaration && !f.target_features.is_empty())
    {
        let features = &func.target_features;
        let mut rules: Vec<(&'static str, String, Scan)> = Vec::new();
        // The lowest removed level: its scan covers the levels above it.
        // x86-64 has no SSE3 class of its own (codegen reaches SSE3 only
        // through SSE4.1, which the vector permission already follows).
        if let Some((entry, unit, scan)) =
            levels
                .iter()
                .take(if i686 { 3 } else { 2 })
                .find_map(|&(feature, unit, scan)| {
                    crate::backend::x86::isa::target_option_denying(
                        tu,
                        tu_explicit,
                        features,
                        !i686,
                        feature,
                    )
                    .map(|entry| (entry, unit, scan))
                })
        {
            rules.push((unit, entry.to_string(), scan));
        }
        if let Some(entry) = crate::backend::x86::isa::function_x87_denial(features) {
            rules.push(("x87", entry.clone(), first_x87_reference));
        }
        if rules.is_empty() {
            continue;
        }
        let body = function_text(asm, &func.name);
        for (unit, entry, scan) in rules {
            if let Some((name, line)) = scan(&body) {
                return Some((name, line, unit, entry));
            }
        }
    }
    None
}

/// The text of function `name`: from its `.type name, @function` line up to
/// the next function's, empty when it was not emitted.
fn function_text(asm: &str, name: &str) -> String {
    fn function_of(line: &str) -> Option<&str> {
        line.trim_start()
            .strip_prefix(".type ")
            .filter(|rest| rest.trim_end().ends_with("@function"))
            .and_then(|rest| rest.split(',').next())
            .map(str::trim)
    }
    let mut out = String::new();
    let mut inside = false;
    for line in asm.lines() {
        if let Some(f) = function_of(line) {
            inside = f == name;
        }
        if inside {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

fn first_sse_reference(asm: &str) -> Option<(String, String)> {
    first_xmm_reference_if(asm, |_, _| true)
}

/// SSE1's XMM vocabulary, including AT&T size-suffixed scalar conversions.
/// User inline assembly remains an explicit escape hatch, as in the no-SSE
/// fence. MMX-only instructions without an XMM operand are not SSE2 leaks.
fn first_sse2_reference(asm: &str) -> Option<(String, String)> {
    first_sse2_reference_unless(asm, |_| false)
}

fn first_sse2_reference_unless(
    asm: &str,
    permitted: impl Fn(&str) -> bool,
) -> Option<(String, String)> {
    first_xmm_reference_if(asm, |function, op| {
        !permitted(function)
            && !matches!(
                op,
                "movss"
                    | "movaps"
                    | "movups"
                    | "movhps"
                    | "movlps"
                    | "movhlps"
                    | "movlhps"
                    | "unpcklps"
                    | "unpckhps"
                    | "shufps"
                    | "movmskps"
                    | "movntps"
                    | "addss"
                    | "addps"
                    | "subss"
                    | "subps"
                    | "mulss"
                    | "mulps"
                    | "divss"
                    | "divps"
                    | "sqrtss"
                    | "sqrtps"
                    | "rcpss"
                    | "rcpps"
                    | "rsqrtss"
                    | "rsqrtps"
                    | "minss"
                    | "minps"
                    | "maxss"
                    | "maxps"
                    | "andps"
                    | "andnps"
                    | "orps"
                    | "xorps"
                    | "comiss"
                    | "ucomiss"
                    | "cmpps"
                    | "cmpss"
                    | "cmpeqps"
                    | "cmpeqss"
                    | "cmpltps"
                    | "cmpltss"
                    | "cmpleps"
                    | "cmpless"
                    | "cmpunordps"
                    | "cmpunordss"
                    | "cmpneqps"
                    | "cmpneqss"
                    | "cmpnltps"
                    | "cmpnltss"
                    | "cmpnleps"
                    | "cmpnless"
                    | "cmpordps"
                    | "cmpordss"
                    | "cvtpi2ps"
                    | "cvtps2pi"
                    | "cvttps2pi"
                    | "cvtsi2ss"
                    | "cvtsi2ssl"
                    | "cvtsi2ssq"
                    | "cvtss2si"
                    | "cvtss2sil"
                    | "cvtss2siq"
                    | "cvttss2si"
                    | "cvttss2sil"
                    | "cvttss2siq"
            )
    })
}

fn first_xmm_reference_if(
    asm: &str,
    forbidden: impl Fn(&str, &str) -> bool,
) -> Option<(String, String)> {
    first_instruction_if(asm, |function, op, code| {
        (code.contains("%xmm") || code.contains("%ymm") || code.contains("%zmm"))
            && forbidden(function, op)
    })
}

/// SSE3's vocabulary (the level above SSE2): FISTTP and the horizontal /
/// duplicate / unaligned-load vector forms. MONITOR/MWAIT are never
/// compiler-chosen.
fn first_sse3_reference(asm: &str) -> Option<(String, String)> {
    first_sse3_reference_unless(asm, |_| false)
}

fn first_sse3_reference_unless(
    asm: &str,
    permitted: impl Fn(&str) -> bool,
) -> Option<(String, String)> {
    first_instruction_if(asm, |function, op, _| {
        !permitted(function)
            && (op.starts_with("fisttp")
                || matches!(
                    op,
                    "addsubps"
                        | "addsubpd"
                        | "haddps"
                        | "haddpd"
                        | "hsubps"
                        | "hsubpd"
                        | "movddup"
                        | "movshdup"
                        | "movsldup"
                        | "lddqu"
                ))
    })
}

/// Any x87 instruction. Every x87 mnemonic (SDM Vol. 2, AT&T suffixes
/// included) begins with `f`; the only other x86 mnemonics that do are the
/// FXSAVE/FXRSTOR state images and 3DNow!'s FEMMS, which compute nothing.
fn first_x87_reference(asm: &str) -> Option<(String, String)> {
    first_instruction_if(asm, |_, op, _| {
        op.starts_with('f')
            && !op.starts_with("fxsave")
            && !op.starts_with("fxrstor")
            && op != "femms"
    })
}

/// The first EMITTED function whose definition, or one of whose calls,
/// returns a value in `%st(0)`: long double everywhere, and on i686 also
/// `float`/`double`. Functions absent from the final text (unused statics)
/// are skipped, like GCC, which only diagnoses what it emits.
fn first_x87_return_boundary(
    module: &crate::ir::module::IrModule,
    asm: &str,
    i686: bool,
) -> Option<(String, String)> {
    use crate::common::types::IrType;
    use crate::ir::instruction::Instruction;
    let in_st0 =
        |ty: &IrType| ty.is_long_double() || (i686 && matches!(ty, IrType::F32 | IrType::F64));
    let emitted = |name: &str| {
        asm.lines().any(|l| {
            l.strip_suffix(':')
                .is_some_and(|label| label.trim() == name)
        })
    };
    for func in module.functions.iter().filter(|f| !f.is_declaration) {
        if !emitted(&func.name) {
            continue;
        }
        if in_st0(&func.return_type) {
            return Some((
                func.name.clone(),
                format!("its {:?} return value", func.return_type),
            ));
        }
        for inst in func.blocks.iter().flat_map(|b| &b.instructions) {
            let what = match inst {
                Instruction::Call { func: callee, info } if in_st0(&info.return_type) => {
                    format!("the call to '{callee}' ({:?} result)", info.return_type)
                }
                Instruction::CallIndirect { info, .. } if in_st0(&info.return_type) => {
                    format!("an indirect call ({:?} result)", info.return_type)
                }
                _ => continue,
            };
            return Some((func.name.clone(), what));
        }
    }
    None
}

/// First compiler-generated instruction line for which `forbidden(function,
/// mnemonic, code)` holds, with its enclosing function. User inline assembly (`#APP`
/// ... `#NO_APP`), comments, directives and labels never count.
fn first_instruction_if(
    asm: &str,
    forbidden: impl Fn(&str, &str, &str) -> bool,
) -> Option<(String, String)> {
    let mut func = String::from("<toplevel>");
    let mut in_app = false;
    for line in asm.lines() {
        let t = line.trim_start();
        if t.starts_with("#APP") {
            in_app = true;
            continue;
        }
        if t.starts_with("#NO_APP") {
            in_app = false;
            continue;
        }
        if in_app || t.is_empty() || t.starts_with('#') {
            continue;
        }
        if t.starts_with('.') {
            if t.starts_with(".size ") {
                func = String::from("<toplevel>");
            }
            if let Some(rest) = t.strip_prefix(".type ") {
                if rest.trim_end().ends_with("@function") {
                    if let Some(name) = rest.split(',').next() {
                        func = name.trim().to_string();
                    }
                }
            }
            continue;
        }
        if t.ends_with(':') {
            continue;
        }
        let code = t.split('#').next().unwrap_or("");
        let op = code.split_ascii_whitespace().next().unwrap_or("");
        if forbidden(&func, op, code) {
            return Some((func, line.to_string()));
        }
    }
    None
}

#[cfg(test)]
mod sse_reference_tests {
    use super::{
        first_sse_reference, first_sse2_reference, first_sse3_reference, first_x87_reference,
        first_x87_return_boundary,
    };

    #[test]
    fn x87_fence_names_every_x87_form_and_nothing_else() {
        let prefix = ".type f, @function\nf:\n";
        for op in [
            "fldl 4(%esp)",
            "fstps (%esp)",
            "fildll (%esp)",
            "fistpq 8(%esp)",
            "fnstcw (%esp)",
            "fucomip %st(1), %st",
            "fxch %st(1)",
            "fld1",
            "fldt (%ecx)",
            "fcmovbe %st(1), %st",
        ] {
            let (func, line) = first_x87_reference(&format!("{prefix}    {op}\n")).unwrap();
            assert_eq!((func.as_str(), line.trim()), ("f", op));
        }
        // Integer code, FP state images, FEMMS, SSE scalar FP, symbols named
        // like x87 mnemonics, labels and user inline assembly never count.
        let clean = ".type fldl, @function\nfldl:\n    movl 4(%esp), %eax\n\
                     fxsave (%eax)\n    fxrstor64 (%rax)\n    femms\n\
                     addsd %xmm1, %xmm0\n    call fabs\n    lock cmpxchg8b (%esi)\n\
                     #APP\n    fldz\n#NO_APP\n    # fstp in a comment\n    ret\n";
        assert!(first_x87_reference(clean).is_none());
    }

    #[test]
    fn target_attribute_denials_are_enforced_on_their_own_function_only() {
        use super::first_target_attribute_leak;
        use crate::common::types::IrType;
        use crate::ir::module::{IrFunction, IrModule};
        let asm = concat!(
            ".type ints, @function\nints:\n    movq %rdi, %rax\n    ret\n",
            ".type flt, @function\nflt:\n    mulss %xmm1, %xmm0\n    ret\n",
            ".type dbl, @function\ndbl:\n    mulsd %xmm1, %xmm0\n    ret\n",
            ".type ld, @function\nld:\n    fmulp %st, %st(1)\n    ret\n",
            ".type user, @function\nuser:\n#APP\n    movaps %xmm0, %xmm1\n#NO_APP\n    ret\n",
        );
        let check = |name: &str, features: &[&str]| {
            let mut f = IrFunction::new(name.into(), IrType::I32, vec![], false);
            f.target_features = features.iter().map(|s| (*s).to_string()).collect();
            let mut module = IrModule::new();
            module.functions = vec![f];
            let tu = crate::backend::x86::isa::X86Isa::arch_row("x86-64")
                .map_or(0, |row| row.feature_bits());
            first_target_attribute_leak(&module, asm, false, tu, 0)
                .map(|(func, line, unit, entry)| (func, line.trim().to_string(), unit, entry))
        };
        assert_eq!(
            check("flt", &["general-regs-only"]),
            Some((
                "flt".into(),
                "mulss %xmm1, %xmm0".into(),
                "SSE",
                "general-regs-only".into()
            ))
        );
        assert_eq!(check("flt", &["no-sse"]).map(|h| h.2), Some("SSE"));
        // GCC 16.2 keeps SSE1 `mulss` under `target("no-sse2")`.
        assert_eq!(check("flt", &["no-sse2"]), None);
        assert_eq!(
            check("dbl", &["no-sse2"]).map(|h| (h.2, h.3)),
            Some(("SSE2", "no-sse2".into()))
        );
        // The options apply in order, as in codegen and GCC: a later denial
        // wins, and a later grant lifts it (`aes` brings SSE2 back).
        assert_eq!(check("dbl", &["avx2", "no-sse"]).map(|h| h.2), Some("SSE"));
        assert_eq!(check("dbl", &["no-sse2", "aes"]), None);
        assert_eq!(check("dbl", &["no-sse", "sse2"]), None);
        assert_eq!(
            check("ld", &["general-regs-only", "80387"]).map(|h| h.2),
            None
        );
        assert_eq!(
            check("ld", &["no-80387"]).map(|h| (h.2, h.3)),
            Some(("x87", "no-80387".into()))
        );
        assert_eq!(
            check("ld", &["general-regs-only"]).map(|h| h.2),
            Some("x87")
        );
        assert_eq!(check("ld", &["no-sse"]), None);
        // Integer code, other functions' SSE, user asm and unemitted
        // functions are never attributed.
        assert_eq!(check("ints", &["general-regs-only"]), None);
        assert_eq!(check("user", &["general-regs-only"]), None);
        assert_eq!(check("gone", &["general-regs-only"]), None);
        assert_eq!(check("flt", &["avx2"]), None);
    }

    /// On i686 a function's own `arch=` is its SSE ceiling (GCC discards
    /// the TU's ISA flags for it), and `no-sse3` is enforced like the
    /// other denials.
    #[test]
    fn i686_target_attribute_arch_sets_the_function_ceiling() {
        use super::first_target_attribute_leak;
        use crate::common::types::IrType;
        use crate::ir::module::{IrFunction, IrModule};
        let asm = concat!(
            ".type flt, @function\nflt:\n    mulss %xmm1, %xmm0\n    ret\n",
            ".type dbl, @function\ndbl:\n    mulsd %xmm1, %xmm0\n    ret\n",
            ".type cvt, @function\ncvt:\n    fisttpl (%esp)\n    ret\n",
        );
        let check = |name: &str, features: &str| {
            let mut f = IrFunction::new(name.into(), IrType::I32, vec![], false);
            f.target_features = crate::ir::module::normalize_target_features(features);
            let mut module = IrModule::new();
            module.functions = vec![f];
            // A TU with SSE3 (`-march=prescott`), so every level can be denied;
            // what the TU lacks is the TU-level scan's to report.
            let tu = crate::backend::x86::isa::X86Isa::arch_row("prescott")
                .map_or(0, |row| row.feature_bits());
            first_target_attribute_leak(&module, asm, true, tu, 0).map(|hit| (hit.2, hit.3))
        };
        assert_eq!(check("flt", "arch=i686"), Some(("SSE", "arch=i686".into())));
        assert_eq!(check("flt", "arch=pentium3"), None);
        assert_eq!(
            check("dbl", "arch=pentium3"),
            Some(("SSE2", "arch=pentium3".into()))
        );
        assert_eq!(check("dbl", "arch=pentium4"), None);
        assert_eq!(
            check("cvt", "arch=pentium4"),
            Some(("SSE3", "arch=pentium4".into()))
        );
        assert_eq!(check("cvt", "arch=prescott"), None);
        // Grants after `arch=` raise the ceiling; ones before it are gone.
        assert_eq!(check("dbl", "arch=i686,sse2"), None);
        assert_eq!(check("dbl", "sse2,arch=i686").map(|h| h.0), Some("SSE"));
        assert_eq!(check("cvt", "no-sse3"), Some(("SSE3", "no-sse3".into())));
        assert_eq!(check("cvt", "sse2"), None);
    }

    #[test]
    fn x87_return_boundary_counts_emitted_definitions_and_calls_only() {
        use crate::common::types::IrType;
        use crate::ir::instruction::{BasicBlock, CallInfo, Instruction, Terminator};
        use crate::ir::module::{IrFunction, IrModule};
        use crate::ir::reexports::BlockId;
        let func = |name: &str, ret: IrType, calls: &[(&str, IrType)]| {
            let mut f = IrFunction::new(name.into(), ret, vec![], false);
            f.blocks.push(BasicBlock {
                label: BlockId(0),
                instructions: calls
                    .iter()
                    .map(|(callee, ty)| Instruction::Call {
                        func: (*callee).into(),
                        info: CallInfo {
                            return_type: *ty,
                            ..CallInfo::default()
                        },
                    })
                    .collect(),
                terminator: Terminator::Unreachable,
                source_spans: Vec::new(),
            });
            f
        };
        let check = |funcs: Vec<IrFunction>, asm: &str, i686: bool| {
            let mut module = IrModule::new();
            module.functions = funcs;
            first_x87_return_boundary(&module, asm, i686)
                .map(|(f, what)| (f, what.contains("call")))
        };
        let asm = "ints:\n    ret\ndbl:\n    ret\nuser:\n    ret\n";
        // i686: float/double/long double all return in %st(0).
        for ty in [IrType::F32, IrType::F64, IrType::F128] {
            assert_eq!(
                check(vec![func("dbl", ty, &[])], asm, true),
                Some(("dbl".into(), false))
            );
            assert_eq!(
                check(vec![func("user", IrType::I32, &[("ext", ty)])], asm, true),
                Some(("user".into(), true))
            );
        }
        // x86-64: only long double does.
        assert_eq!(check(vec![func("dbl", IrType::F64, &[])], asm, false), None);
        assert_eq!(
            check(
                vec![func("user", IrType::I32, &[("ext", IrType::F128)])],
                asm,
                false
            ),
            Some(("user".into(), true))
        );
        // Integer boundaries, and FP functions never emitted (unused statics).
        assert_eq!(
            check(
                vec![
                    func("ints", IrType::I64, &[("ext", IrType::I32)]),
                    func("unused", IrType::F64, &[("ext", IrType::F64)]),
                ],
                asm,
                true
            ),
            None
        );
    }

    #[test]
    fn sse3_fence_names_fisttp_and_the_sse3_vector_forms_only() {
        let prefix = ".type f, @function\nf:\n";
        for op in [
            "fisttpl (%esp)",
            "fisttpq 4(%esp)",
            "fisttps (%esp)",
            "haddps %xmm1, %xmm0",
            "movddup (%eax), %xmm0",
            "lddqu (%eax), %xmm1",
        ] {
            let (func, line) = first_sse3_reference(&format!("{prefix}    {op}\n")).unwrap();
            assert_eq!(func, "f");
            assert_eq!(line.trim(), op);
        }
        // The SSE3-free equivalents, SSE2 forms, symbols that merely
        // contain the mnemonic, and user inline assembly never count.
        let clean = ".type fisttp, @function\nfisttp:\n    fistpl 4(%esp)\n\
                     fnstcw (%esp)\n    cvttsd2sil 4(%esp), %eax\n\
                     addps %xmm1, %xmm0\n    call fisttp\n#APP\n    fisttpl (%esp)\n\
                     #NO_APP\n    # fisttpl in a comment\n    ret\n";
        assert!(first_sse3_reference(clean).is_none());
    }

    #[test]
    fn sse1_fence_keeps_legal_memory_arithmetic_and_rejects_sse2() {
        let prefix = ".type f, @function\nf:\n";
        for op in [
            "movss (%rdi), %xmm0",
            "addps %xmm1, %xmm0",
            "xorps %xmm0, %xmm0",
            "cvtsi2ssq %rax, %xmm0",
            "ucomiss %xmm1, %xmm0",
            "movq %mm0, %mm1",
        ] {
            assert!(
                first_sse2_reference(&format!("{prefix}{op}\n")).is_none(),
                "{op}"
            );
        }
        for op in [
            "movdqu (%rdi), %xmm0",
            "pmuludq %xmm1, %xmm0",
            "addsd %xmm1, %xmm0",
            "movd %eax, %xmm0",
            "vaddps %xmm1, %xmm0, %xmm0",
        ] {
            let (name, _) = first_sse2_reference(&format!("{prefix}{op}\n")).unwrap();
            assert_eq!(name, "f", "{op}");
        }
        assert!(
            first_sse2_reference(&format!(
                "{prefix}#APP\naddsd %xmm1,%xmm0\n#NO_APP\nret # %xmm0\n"
            ))
            .is_none()
        );
    }

    #[test]
    fn finds_first_sse_line_with_function_name() {
        let asm = ".type f, @function\nf:\n    movq %rdi, %rax\n    ret\n\
                   .type g, @function\ng:\n    # LCCC_RET_XMM 1\n    movsd (%rdi), %xmm0\n    ret\n";
        let (f, l) = first_sse_reference(asm).unwrap();
        assert_eq!(f, "g");
        assert_eq!(l.trim(), "movsd (%rdi), %xmm0");
    }

    #[test]
    fn inline_asm_and_comments_are_exempt() {
        let asm = ".type f, @function\nf:\n#APP\n    movaps %xmm0, %xmm1\n#NO_APP\n\
                   # note: %xmm0\n    ret\n";
        assert!(first_sse_reference(asm).is_none());
    }
}

#[cfg(test)]
mod loop_idiom_capability_tests {
    use super::*;

    #[test]
    fn native_popcount_permission_obeys_every_target_and_explicit_veto() {
        let mut driver = Driver::new();
        // No `-march=` state is set up here on purpose: an explicit profile
        // reaches this predicate only through `enable_popcnt` (the profile
        // sets it, `-mno-popcnt` clears it and marks the sticky denial), so
        // driving the two flags directly covers every profile outcome.
        for target in [Target::X86_64, Target::I686] {
            driver.target = target;
            driver.enable_popcnt = false;
            assert!(!driver.resolved_fast_popcount());
            driver.enable_popcnt = true;
            assert!(driver.resolved_fast_popcount());
            driver.popcnt_explicitly_disabled = true;
            assert!(!driver.resolved_fast_popcount());
            driver.popcnt_explicitly_disabled = false;
        }
        driver.target = Target::Aarch64;
        driver.general_regs_only = false;
        assert!(driver.resolved_fast_popcount());
        driver.general_regs_only = true;
        assert!(!driver.resolved_fast_popcount());
        driver.target = Target::Riscv64;
        driver.general_regs_only = false;
        assert!(!driver.resolved_fast_popcount());
    }
}

#[cfg(test)]
mod scoped_sse2_fence_tests {
    use super::first_sse2_reference_unless;

    #[test]
    fn local_grant_does_not_exempt_a_neighbor_or_toplevel() {
        let prefix =
            ".type allowed, @function\nallowed:\n paddd %xmm1, %xmm0\n.size allowed, .-allowed\n";
        assert!(first_sse2_reference_unless(prefix, |name| name == "allowed").is_none());
        let neighbor = format!("{prefix}.type denied, @function\ndenied:\n paddd %xmm1, %xmm0\n");
        assert_eq!(
            first_sse2_reference_unless(&neighbor, |name| name == "allowed")
                .unwrap()
                .0,
            "denied"
        );
        let outside = format!("{prefix} movdqu %xmm0, %xmm1\n");
        assert_eq!(
            first_sse2_reference_unless(&outside, |name| name == "allowed")
                .unwrap()
                .0,
            "<toplevel>"
        );
    }
}

#[cfg(test)]
mod i686_target_grant_tests {
    use super::first_sse3_reference_unless;

    /// `[SSE, SSE2, SSE3]` of an i686 function with `target(features)` in a
    /// TU without SSE (`-march=i386`).
    fn grants(features: &[&str]) -> [bool; 3] {
        use crate::common::x86_features::X86Feature as F;
        let owned: Vec<String> = features.iter().map(|f| (*f).to_string()).collect();
        let bits = crate::backend::x86::isa::function_feature_bits(0, 0, &owned, false);
        [F::Sse, F::Sse2, F::Sse3].map(|f| bits & (1 << f as u8) != 0)
    }

    /// GCC 16.2 `-m32 -march=i386`, `#pragma GCC target(...)`: grants imply
    /// the levels below them and the options apply in order (measured).
    #[test]
    fn grants_imply_lower_levels_in_order() {
        assert_eq!(grants(&[]), [false; 3]);
        assert_eq!(grants(&["sse"]), [true, false, false]);
        assert_eq!(grants(&["sse2"]), [true, true, false]);
        assert_eq!(grants(&["ssse3"]), [true; 3]);
        assert_eq!(grants(&["avx2"]), [true; 3]);
        assert_eq!(grants(&["no-sse3", "sse4.1"]), [true; 3]);
        assert_eq!(grants(&["no-sse2", "sse4.1"]), [true; 3]);
        assert_eq!(grants(&["avx", "no-sse"]), [false; 3]);
        assert_eq!(grants(&["general-regs-only", "sse2"]), [true, true, false]);
        assert_eq!(grants(&["sse2", "general-regs-only"]), [false; 3]);
        // Integer extensions grant no SSE.
        assert_eq!(grants(&["popcnt", "bmi2"]), [false; 3]);
    }

    #[test]
    fn arch_grants_the_cpus_levels() {
        assert_eq!(grants(&["arch=x86-64"]), [true, true, false]);
        assert_eq!(grants(&["arch=nehalem"]), [true; 3]);
        assert_eq!(grants(&["arch=haswell"]), [true; 3]);
        assert_eq!(grants(&["arch=pentium"]), [false; 3]);
        assert_eq!(grants(&["arch=i686"]), [false; 3]);
        assert_eq!(grants(&["sse3", "arch=i686"]), [false; 3]);
        assert_eq!(grants(&["arch=haswell", "no-sse3"]), [true, true, false]);
        // An unknown name (diagnosed at lowering) grants nothing on i686.
        assert_eq!(grants(&["arch=bogus"]), [false; 3]);
    }

    #[test]
    fn sse3_grant_is_function_scoped() {
        let asm = ".type ok, @function\nok:\n haddps %xmm1, %xmm0\n.size ok, .-ok\n\
                   .type bad, @function\nbad:\n fisttpl (%esp)\n";
        assert_eq!(
            first_sse3_reference_unless(asm, |f| f == "ok").unwrap().0,
            "bad"
        );
        assert!(first_sse3_reference_unless(asm, |f| f == "ok" || f == "bad").is_none());
    }
}
