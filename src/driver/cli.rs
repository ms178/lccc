//! CLI argument parsing for GCC-compatible command-line flags.
//!
//! Handles the full range of GCC flags that build systems like the Linux kernel,
//! Meson, and autoconf expect: optimization levels, debug info, preprocessor
//! directives, linker flags, target-specific machine flags, and query flags
//! like `--dumpmachine` and `--version`.
//!
//! Design: The parser is a simple `while` loop with a flat `match` on each
//! argument. No external parser library is used.
//!
//! # Unknown-option policy
//!
//! Two facts pull in opposite directions, and the policy is where they meet.
//!
//! * Garbage must not read as support.  Feature probes (`cc-option`, Meson
//!   `has_argument`, Kconfig) decide by **exit status**, so a flag LCCC does
//!   not implement must not succeed: that turns "does the compiler support
//!   this?" into a false yes, and the build then proceeds under assumptions
//!   the compiler never agreed to.
//! * Real builds must keep building.  Measured against a prebuilt driver, 16
//!   of the 93 flag spellings that the Linux kernel, glibc, zlib-ng, gzip and
//!   expat build systems pass *unconditionally* had no arm here
//!   (`-fno-strict-aliasing`, `-fwrapv`, `-fno-strict-overflow`, `-fno-plt`,
//!   `-flto`, `-funroll-loops`, `-Og`, `-ansi`, `-fno-ident`, `-fuse-ld=*`,
//!   `-fvar-tracking-assignments`, …).  Refusing those breaks real builds.
//!
//! So the policy is **tiered by consequence**, and every tier is measured:
//!
//! | input | behaviour |
//! |---|---|
//! | `-std=<unrecognized>` | **hard error** with a suggestion (a wrong dialect silently mis-parses headers) |
//! | `-std=<known>`, `-ansi` | implemented: `-ansi` is `-std=c90` (measured on GCC: no `__STDC_VERSION__`, `__STRICT_ANSI__` defined) |
//! | *contract* flags — `-fstack-protector*`, `-ftrapv`, `-fsanitize=` | **hard error** + remediation hint (upstream's original rule) |
//! | data-model / ABI requests — `-fshort-enums`, `-fpack-struct[=n]` | **hard error**, because ignoring one silently returns objects whose layout disagrees with the libraries they link against. `-funsigned-char` and `-fshort-wchar` are *not* here: both are implemented (`char_is_unsigned()` / `set_short_wchar`), and `-fsigned-char` names the x86-64 SysV default LCCC already emits |
//! | `-g<selector>` that GNU rejects (`-g4`, `-ggdb9`, `-gdwarf-9`, `-gz=bogus`, `-gno-bogus`, `-gbogus`) | **hard error** — the selector grammar is closed and was measured exhaustively, so a name outside it is a typo, not a newer GNU (GCC 14.2/16.2, Clang 23.1 and ICX all reject these) |
//! | *off-requests* — `-Wno-<unknown>`, `-gno-<feature>` | **accepted silently**: the thing asked to be switched off does not exist here, so the requested state is already the state (all four reference compilers accept `-Wno-<unknown>`, measured) |
//! | everything else unimplemented — `-f*`, `-m*`, `--*`, `-W<name>`, `-Werror=<name>`, `--param`, GNU-accepted `-g` selectors, presentation namespaces (`-fdiagnostics-*`, `-fmessage-length=`, `-fmax-errors=`, `-f*prefix-map=`, `-fverbose-asm`) | **warning naming the option**, once per compilation; `LCCC_STRICT_OPTIONS=1` makes it fatal |
//!
//! The last row is the one that needs defending, and the defence is the
//! measurement, not taste.  LCCC implements six warning names out of the 409
//! GNU exposes (`gcc -Q --help=warnings`, measured) and has no parameter,
//! debug-selector, optimisation or long-option table at all, so for those
//! namespaces it cannot tell a typo from a spelling a newer GNU added.  The
//! consequence of ignoring a request in this row is an optimisation, warning,
//! reporting or debug-info preference *not being applied* — visible in the
//! diagnostic — rather than a program that means something different, which is
//! why the meaning-changing requests above are refused instead.  Strict mode
//! exists precisely because a capability probe cannot live with that judgement
//! call: under `LCCC_STRICT_OPTIONS=1` every tolerated request fails, so the
//! exit status means "implemented", and that is the mode
//! `scripts/lccc_capability_probe.py` runs in.
//!
//! Escalation is spelled with the `LCCC_STRICT_` prefix already established by
//! `LCCC_STRICT_MFLAGS` below.
//!
//! Two spellings with an implementation in another layer are recognised here
//! so they cannot reach the fallback at all: `--sysroot[=]`/`-isysroot[=]`
//! (resolved by the link layer and the include search) and the GNU link-file
//! switches `-nostartfiles`/`-nodefaultlibs` (recorded on the driver and
//! forwarded by `build_linker_args` to `backend::common`, like `-nostdlib`).

use super::pipeline::{CliDefine, CompileMode, Driver};
use crate::backend::Target;
use crate::common::error::ColorMode;
use crate::common::fp_contract::FpContract;
use crate::passes::loop_align::AlignControl;

/// Validate `-mtls-dialect=` for the final target. GCC's value sets are
/// target-specific (x86: gnu|gnu2, AArch64: trad|desc, RISC-V: trad|desc); a
/// value outside the set gets GCC 16.2's "unrecognized argument" error (with
/// its list of valid arguments), a valid value other than
/// the dialect LCCC emits is refused like every other unimplemented
/// target-affecting `-m` option rather than silently producing the other one.
fn check_tls_dialect(target: &Target, dialect: &str) -> Result<(), String> {
    let (valid, native): (&[&str], &str) = match target {
        Target::X86_64 | Target::I686 => (&["gnu", "gnu2"], "gnu2"),
        Target::Aarch64 => (&["trad", "desc"], "desc"),
        Target::Riscv64 => (&["trad", "desc"], "trad"),
    };
    if dialect == native {
        Ok(())
    } else if valid.contains(&dialect) {
        Err(format!(
            "unsupported machine option -mtls-dialect={dialect}; LCCC emits the dynamic \
             TLS models only as -mtls-dialect={native} on this target"
        ))
    } else {
        Err(format!(
            "unrecognized argument in option '-mtls-dialect={dialect}'; valid arguments to \
             '-mtls-dialect=' are: {}",
            valid.join(" ")
        ))
    }
}

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

/// The path of a `-fprofile-*=PATH` option; GCC 16.2 rejects an empty one
/// with this wording instead of falling back to the current directory.
fn profile_path_arg(arg: &str, prefix: &str) -> Result<String, String> {
    match &arg[prefix.len()..] {
        "" => Err(format!("missing argument to '{prefix}'")),
        path => Ok(path.to_string()),
    }
}

/// Whether `-march=<march>` implies PRFCHW (`prefetchw`). The table is GCC's
/// measured `__PRFCHW__` set for every x86 `-march=` this driver accepts
/// (`gcc-16 -march=X -dM -E`; `knl`/`knm` from GCC 14, which still accepts
/// them): Broadwell and later big cores, Silvermont and later Atoms, every
/// Zen. Notably absent: Nehalem..Haswell and the `x86-64-v2/v3/v4` levels
/// (the ISA levels do not include PRFCHW). `native` is probed separately.
fn x86_march_implies_prfchw(march: &str) -> bool {
    matches!(
        march,
        "silvermont"
            | "goldmont"
            | "broadwell"
            | "skylake"
            | "znver1"
            | "znver2"
            | "znver3"
            | "znver4"
            | "znver5"
            | "alderlake"
            | "raptorlake"
            | "raptor-lake"
            | "meteorlake"
            | "gracemont"
            | "alderlake-n"
            | "arrowlake"
            | "sierraforest"
            | "grandridge"
            | "arrowlake-s"
            | "lunarlake"
            | "wildcatlake"
            | "clearwaterforest"
            | "skylake-avx512"
            | "cascadelake"
            | "cooperlake"
            | "icelake-client"
            | "icelake-server"
            | "tigerlake"
            | "rocketlake"
            | "sapphirerapids"
            | "graniterapids"
            | "graniterapids-d"
            | "knl"
            | "knm"
    )
}

impl Driver {
    /// Parse the value of `-falign-*=N[:M]`: a power-of-two byte alignment
    /// with an optional max-skip (`.p2align` third operand). Returns `None`
    /// for anything GCC would reject at option processing.
    fn parse_align_spec(value: &str) -> Option<AlignControl> {
        let (align_str, skip_str) = match value.split_once(':') {
            Some((a, s)) => (a, Some(s)),
            None => (value, None),
        };
        let align: u32 = align_str.trim().parse().ok()?;
        // GCC rejects non-power-of-two and absurd alignments here.
        if !align.is_power_of_two() || align > 1 << 16 {
            return None;
        }
        let max_skip = match skip_str {
            None => None,
            Some(s) => {
                let m: u32 = s.trim().parse().ok()?;
                if m > align {
                    // A max-skip at or above the alignment can never skip;
                    // accept it (GAS semantics) but normalize away the
                    // no-op clause so the directive list stays canonical.
                    None
                } else {
                    Some(m)
                }
            }
        };
        Some(AlignControl::Custom { align, max_skip })
    }

    /// Reset the x86-64 arch-IMPLIED ISA surface to the x86-64 baseline
    /// (SSE2-only: every optional `enable_*` feature off).  Called at every
    /// `-march=` — the new profile must REPLACE the previous arch's set, not
    /// union onto it (GCC: `-march=x86-64-v3 -march=x86-64` leaves `__BMI__`,
    /// `__MOVBE__` and `__AVX__` all absent).  Explicit decisions are NOT
    /// lost: the sticky `*_explicitly_disabled` denials and `no_sse` are
    /// resolve-time state and survive, and the recorded explicit flags are
    /// replayed after the profile lands (see
    /// [`Driver::replay_x86_explicit_isa_flags`]).
    fn reset_x86_arch_implied_isa(&mut self) {
        // `enable_sse` is set only by explicit SSE/SSE2-granting flags,
        // which are replayed after the profile like every other explicit
        // decision; `enable_prfchw` is arch-implied or explicit. The sticky
        // denials (`no_sse`, `sse_explicitly_disabled`, `sse2_explicitly_disabled`,
        // `no_sse2`) and `march_pre_sse` are NOT reset here: the denials
        // survive every `-march=` (GCC keeps them in either flag order), and
        // `march_pre_sse` is re-set by the `-march=` handler that this reset
        // serves.
        self.enable_sse = false;
        self.enable_prfchw = false;
        self.enable_sse3 = false;
        self.enable_ssse3 = false;
        self.enable_sse4_1 = false;
        self.enable_sse4_2 = false;
        self.enable_avx = false;
        self.enable_avx2 = false;
        self.enable_aes = false;
        self.enable_pclmul = false;
        self.enable_f16c = false;
        self.enable_fma = false;
        self.enable_bmi = false;
        self.enable_bmi2 = false;
        self.enable_lzcnt = false;
        self.enable_popcnt = false;
        self.enable_movbe = false;
        self.enable_cx16 = false;
        self.enable_rdrnd = false;
        self.enable_avx512f = false;
        self.enable_avx512cd = false;
        self.enable_avx512dq = false;
        self.enable_avx512bw = false;
        self.enable_avx512vl = false;
        self.enable_avx512ifma = false;
        self.enable_avx512vbmi = false;
        self.enable_avx512vbmi2 = false;
        self.enable_avx512vnni = false;
        self.enable_avx512bitalg = false;
        self.enable_avx512vpopcntdq = false;
        self.enable_avx512bf16 = false;
        self.enable_avx512fp16 = false;
        self.enable_avx512er = false;
        self.enable_avx512pf = false;
        self.enable_avx512vp2intersect = false;
        self.enable_avxvnni = false;
        self.enable_avxifma = false;
        self.enable_avxneconvert = false;
        self.enable_avx10_1 = false;
        self.enable_avx10_2 = false;
        self.enable_gfni = false;
        self.enable_vaes = false;
        self.enable_vpclmulqdq = false;
        self.enable_avxvnniint8 = false;
        self.enable_avxvnniint16 = false;
        self.enable_sha512 = false;
        self.enable_sm3 = false;
        self.enable_sm4 = false;
        self.enable_movrs = false;
        self.enable_user_msr = false;
        self.enable_apxf = false;
        self.enable_amx_tile = false;
        self.enable_amx_int8 = false;
        self.enable_amx_bf16 = false;
        self.enable_cmpccxadd = false;
    }

    /// Re-apply the recorded explicit x86 ISA flags, in command-line order,
    /// AFTER a `-march=` profile has landed.  This is what makes an explicit
    /// `-m*`/`-mno-*` decision override the final arch in BOTH directions
    /// (GCC, measured on gcc 14.2: `-mno-bmi -march=x86-64-v3` → `__BMI__`
    /// absent; `-mbmi -march=x86-64` → `__BMI__` defined) while a later
    /// `-march=` still REPLACES the arch-implied set.  Recording happens in
    /// the main argument loop only, so a replay cannot duplicate entries.
    fn replay_x86_explicit_isa_flags(&mut self) {
        let recorded = self.x86_explicit_isa_args.clone();
        for arg in &recorded {
            self.apply_x86_isa_mflag(arg);
        }
    }

    /// The AVX-512 subsets and AVX-VNNI (AVX-512 / AVX10 feature flags;
    /// backend coverage is partial and runtime dispatch must verify the host).
    /// Each implies AVX2 and, for the AVX-512 subsets, AVX-512F, as GCC 16.2
    /// implies them (measured). Returns `false` for any other flag.
    fn grant_avx512_family(&mut self, flag: &str) -> bool {
        let slot = match flag {
            "-mavx512f" => &mut self.enable_avx512f,
            "-mavx512cd" => &mut self.enable_avx512cd,
            "-mavx512dq" => &mut self.enable_avx512dq,
            "-mavx512bw" => &mut self.enable_avx512bw,
            "-mavx512vl" => &mut self.enable_avx512vl,
            "-mavx512ifma" => &mut self.enable_avx512ifma,
            "-mavx512vbmi" => &mut self.enable_avx512vbmi,
            "-mavx512vbmi2" => &mut self.enable_avx512vbmi2,
            "-mavx512vnni" => &mut self.enable_avx512vnni,
            "-mavx512bitalg" => &mut self.enable_avx512bitalg,
            "-mavx512vpopcntdq" => &mut self.enable_avx512vpopcntdq,
            "-mavx512bf16" => &mut self.enable_avx512bf16,
            "-mavx512fp16" => &mut self.enable_avx512fp16,
            "-mavx512er" => &mut self.enable_avx512er,
            "-mavx512pf" => &mut self.enable_avx512pf,
            "-mavx512vp2intersect" => &mut self.enable_avx512vp2intersect,
            "-mavxvnni" => &mut self.enable_avxvnni,
            _ => return false,
        };
        *slot = true;
        if flag != "-mavxvnni" {
            self.enable_avx512f = true;
        }
        self.grant_avx2_class();
        true
    }

    /// Apply one explicit x86 ISA `-m`/`-mno-` flag.  Returns `true` when
    /// `arg` is one of the flags owned here; the caller (the main argument
    /// loop) then records it for [`Driver::replay_x86_explicit_isa_flags`].
    /// This is the SINGLE implementation of every ISA flag's effect — the
    /// immediate application and the post-`-march=` replay run the same
    /// code, so they cannot drift. General-regs-only participates because
    /// later explicit SIMD grants may lift it. Non-ISA `-m` flags
    /// (`-mcmodel=`, `-mtune=`, `-mno-red-zone`, `-mno-80387`,
    /// `-malign-data=`, `-mharden-sls=`, the AVX10 not-implemented errors,
    /// ...) return `false` and stay with the main match: they have no
    /// arch-set interaction.
    fn apply_x86_isa_mflag(&mut self, arg: &str) -> bool {
        if self.grant_avx512_family(arg) || self.apply_x86_sse_level_mflag(arg) {
            return true;
        }
        match arg {
            "-mgeneral-regs-only" => self.deny_general_registers(),
            "-mno-avx2" => {
                self.enable_avx2 = false;
                // AVX2 is the 256-bit integer class — a strict subset
                // of the VEX encoding. Denying it removes `ymm` code
                // but NOT the VEX.128 world: `-march=x86-64-v3
                // -mno-avx2` (the AVX1+FMA target class, GCC's
                // `-march=corei7-avx` relatives) keeps `vmovsd`,
                // `vaddsd` and every scalar FMA family. Killing
                // `avx` here (the pre-fix behavior) made `-mno-avx2
                // -mfma` decline the FMA fold entirely: two `xorpd`
                // and a libm call where GCC emits one `vfnmsub132sd`.
                self.avx2_explicitly_disabled = true;
                self.enable_avxvnni = false;
                self.enable_avxvnniint8 = false;
                self.enable_avxvnniint16 = false;
            }
            "-mno-avx" => {
                self.enable_avx = false;
                self.enable_avx2 = false;
                // `vfmadd*` is VEX-encoded: no AVX means no FMA3 either
                // (GCC rejects -mfma -mno-avx for the same reason).
                self.avx_explicitly_disabled = true;
                self.fma_explicitly_disabled = true;
                self.enable_avxvnni = false;
                self.enable_vaes = false;
                self.enable_vpclmulqdq = false;
            }
            "-mno-sse3" | "-mno-ssse3" | "-mno-sse4" | "-mno-sse4.1" => {
                // SSE4.1 ⊃ SSSE3 ⊃ SSE3 and AVX ⊃ SSE4.2 ⊃ SSE4.1, so
                // denying any of these denies SSE4.1 *and* AVX/FMA.
                self.sse41_explicitly_disabled = true;
                self.avx_explicitly_disabled = true;
                self.fma_explicitly_disabled = true;
                self.enable_sse3 = false;
                self.enable_ssse3 = false;
                self.enable_sse4_1 = false;
                self.enable_sse4_2 = false;
                self.enable_avx = false;
                self.enable_avx2 = false;
            }
            "-mno-sse4.2" => {
                // AVX implies SSE4.2, so denying SSE4.2 denies AVX/FMA;
                // SSE4.1 stays legal (GCC: -mno-sse4.2 keeps -msse4.1).
                self.avx_explicitly_disabled = true;
                self.fma_explicitly_disabled = true;
                self.enable_sse4_2 = false;
                self.enable_avx = false;
                self.enable_avx2 = false;
            }
            // Positive SIMD feature flags: define corresponding macros.
            // -mavx2 implies -mavx implies -msse4.2 implies -msse4.1 implies
            // -mssse3 implies -msse3 (matching GCC's implication chain).
            "-maes" => {
                self.enable_aes = true;
                self.grant_sse2_class();
                self.enable_sse = true;
            }
            "-mpclmul" => {
                self.enable_pclmul = true;
                self.grant_sse2_class();
                self.enable_sse = true;
            }
            // F16C, VAES and VPCLMULQDQ are VEX-encoded: GCC 16.2 implies
            // `__AVX__` and `__SSE4_2__` for each (measured), so each grants
            // the AVX class as `-mavx` does. None implies AVX2.
            "-mf16c" => {
                self.enable_f16c = true;
                self.grant_avx_class();
            }
            "-mfma" => {
                // FMA3 exists only in the VEX encoding — there is no
                // legacy-SSE FMA — so -mfma implies -mavx and its full
                // prerequisite chain, exactly like GCC's one-directional
                // implication (oracle, gcc 14.2 -Q --help=target:
                // `-march=x86-64 -mfma` reports avx, sse4.1, sse4.2 all
                // ENABLED with avx2 still disabled; `-mavx` alone leaves
                // fma disabled). Later-flag-wins keeps both directions:
                // a later -mno-avx/-mno-sse4.1/-mno-sse still kills the
                // chain (their arms set the denial flags the ceiling
                // consults), and -mfma after -mno-sse re-enables the
                // vector file (gcc: -mno-sse -mfma -> sse enabled;
                // -mfma -mno-sse -> everything off).
                self.grant_sse2_class();
                self.sse41_explicitly_disabled = false;
                self.avx_explicitly_disabled = false;
                self.enable_avx = true;
                self.enable_sse4_2 = true;
                self.enable_sse4_1 = true;
                self.enable_ssse3 = true;
                self.enable_sse3 = true;
                self.enable_fma = true;
                self.fma_explicitly_disabled = false;
                // FMA ⇒ AVX ⊃ SSE4.2 ⇒ POPCNT (GCC, measured on
                // gcc 14.2: `-mfma -march=x86-64` defines `__POPCNT__`).
                self.enable_popcnt = true;
            }
            // Last-explicit ISA decision wins (GCC): a later `-mbmi`
            // lifts an earlier `-mno-bmi` sticky denial.
            "-mbmi" => {
                self.enable_bmi = true;
                self.bmi_explicitly_disabled = false;
            }
            "-mbmi2" => {
                self.enable_bmi2 = true;
                self.bmi2_explicitly_disabled = false;
            }
            "-mlzcnt" => {
                // Last-explicit ISA decision wins (GCC): a later
                // `-mlzcnt` lifts an earlier `-mno-lzcnt` sticky denial.
                self.enable_lzcnt = true;
                self.lzcnt_explicitly_disabled = false;
            }
            "-mpopcnt" => {
                // Last-explicit ISA decision wins (GCC): a later
                // `-mpopcnt` lifts an earlier `-mno-popcnt` sticky
                // denial.
                self.enable_popcnt = true;
                self.popcnt_explicitly_disabled = false;
            }
            "-mcx16" => {
                self.enable_cx16 = true;
                self.cx16_explicitly_disabled = false;
            }
            "-mmovbe" => {
                self.enable_movbe = true;
                self.movbe_explicitly_disabled = false;
            }
            "-mrdrnd" => self.enable_rdrnd = true,
            "-mavxifma" => {
                self.enable_avxifma = true;
                self.grant_avx2_class();
            }
            "-mavxneconvert" => {
                self.enable_avxneconvert = true;
                self.grant_avx2_class();
            }
            "-mgfni" => self.enable_gfni = true,
            "-mavxvnniint8" => {
                self.enable_avxvnniint8 = true;
                self.grant_avx2_class();
            }
            "-mavxvnniint16" => {
                self.enable_avxvnniint16 = true;
                self.grant_avx2_class();
            }
            "-msha512" => {
                self.enable_sha512 = true;
                self.grant_avx_class();
            }
            "-msm3" => {
                self.enable_sm3 = true;
                self.grant_avx_class();
            }
            "-msm4" => {
                self.enable_sm4 = true;
                self.grant_avx_class();
            }
            "-mmovrs" => self.enable_movrs = true,
            "-muser_msr" => self.enable_user_msr = true,
            // APX Foundation: extra GPRs r16–r31, NDD 3-operand ALU.
            // Off by default — emitting it on a non-APX host (Raptor Lake,
            // i7-14700KF, …) is #UD. `-march=raptorlake` must never imply this.
            "-mapx" | "-mapxf" => self.enable_apxf = true,
            "-mno-apx" | "-mno-apxf" => self.enable_apxf = false,
            "-mamx-tile" => self.enable_amx_tile = true,
            "-mamx-int8" => self.enable_amx_int8 = true,
            "-mamx-bf16" => self.enable_amx_bf16 = true,
            "-mcmpccxadd" => self.enable_cmpccxadd = true,
            "-mno-avxvnni" => self.enable_avxvnni = false,
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
            "-mvaes" => {
                self.enable_vaes = true;
                self.grant_avx_class();
            }
            "-mvpclmulqdq" => {
                self.enable_vpclmulqdq = true;
                self.grant_avx_class();
            }
            "-mno-aes" => self.enable_aes = false,
            "-mno-pclmul" => self.enable_pclmul = false,
            "-mno-f16c" => self.enable_f16c = false,
            "-mno-fma" => {
                self.enable_fma = false;
                self.fma_explicitly_disabled = true;
            }
            "-mno-bmi" => {
                self.enable_bmi = false;
                // Sticky denial: a later -march=x86-64-v3 grant carries
                // BMI1; this explicit denial removes it from that grant (GCC keeps the denial
                // sticky across a later `-march=native` probe too).
                self.bmi_explicitly_disabled = true;
            }
            "-mno-bmi2" => {
                self.enable_bmi2 = false;
                self.bmi2_explicitly_disabled = true;
            }
            "-mno-lzcnt" => {
                self.enable_lzcnt = false;
                // Sticky denial: an explicit -march=x86-64-v3 grant carries
                // ABM (LZCNT/TZCNT); this explicit denial removes it from that grant.
                self.lzcnt_explicitly_disabled = true;
            }
            "-mno-popcnt" => {
                self.enable_popcnt = false;
                // Sticky denial: an explicit v2+ grant carries POPCNT;
                // this explicit denial removes it from that grant.
                self.popcnt_explicitly_disabled = true;
            }
            "-mno-cx16" => {
                self.enable_cx16 = false;
                self.cx16_explicitly_disabled = true;
            }
            "-mno-movbe" => {
                self.enable_movbe = false;
                self.movbe_explicitly_disabled = true;
            }
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
            "-mno-vpclmulqdq" => self.enable_vpclmulqdq = false,
            "-mavx2" => self.grant_avx2_class(),
            "-mavx" => self.grant_avx_class(),
            "-msse4.2" => {
                self.grant_sse2_class();
                self.sse41_explicitly_disabled = false;
                self.enable_sse4_2 = true;
                self.enable_sse4_1 = true;
                self.enable_ssse3 = true;
                self.enable_sse3 = true;
                // SSE4.2 ⇒ POPCNT (GCC, measured on gcc 14.2: `-msse4.2
                // -march=x86-64` defines `__POPCNT__`; SSE4.1 does NOT).
                self.enable_popcnt = true;
            }
            "-msse4.1" | "-msse4" => {
                self.grant_sse2_class();
                self.sse41_explicitly_disabled = false;
                self.enable_sse4_1 = true;
                self.enable_ssse3 = true;
                self.enable_sse3 = true;
            }
            "-mssse3" => {
                self.grant_sse2_class();
                self.enable_ssse3 = true;
                self.enable_sse3 = true;
            }
            "-msse3" => {
                self.grant_sse2_class();
                self.enable_sse3 = true;
            }
            "-mprfchw" => {
                // Last explicit decision wins (GCC): lifts an earlier
                // `-mno-prfchw`.
                self.enable_prfchw = true;
                self.prfchw_explicitly_disabled = false;
            }
            "-mno-prfchw" => {
                // Sticky against a later PRFCHW `-march=` (GCC 16, measured:
                // `-mno-prfchw -march=raptorlake` keeps `prefetcht0` for
                // write hints).
                self.enable_prfchw = false;
                self.prfchw_explicitly_disabled = true;
            }
            _ => return false,
        }
        true
    }

    /// Apply the scalar SSE/SSE2 level flags in command-line order. Keeping
    /// the two denial bits independent is what lets i386 retain an SSE XMM
    /// ABI under `-msse -mno-sse2` and restore SSE2 only after an SSE2-class
    /// grant. Returns `false` for flags outside this family.
    fn apply_x86_sse_level_mflag(&mut self, arg: &str) -> bool {
        match arg {
            "-mno-sse" | "-mno-sse2" => {
                // SSE2 denial also removes every higher vector class. Only
                // `-mno-sse` removes an independent SSE grant. Final target
                // resolution preserves x86-64's existing no-SSE2 path.
                let all_sse = arg == "-mno-sse";
                self.no_sse2 = true;
                self.enable_sse2 = false;
                if all_sse {
                    self.no_sse = true;
                    self.sse_explicitly_disabled = true;
                    self.enable_sse = false;
                } else {
                    self.sse2_explicitly_disabled = true;
                }
                self.avx_explicitly_disabled = true;
                self.fma_explicitly_disabled = true;
                self.sse41_explicitly_disabled = true;
                self.enable_sse3 = false;
                self.enable_ssse3 = false;
                self.enable_sse4_1 = false;
                self.enable_sse4_2 = false;
                self.enable_avx = false;
                self.enable_avx2 = false;
            }
            // Baseline x86-64 ISA flags. SSE2/MMX are the x86-64 default,
            // but `-msse`/`-msse2` after `-mno-sse` must re-enable SSE
            // (GCC last-explicit-ISA-flag wins; `-march=native` does not).
            "-msse2" => {
                self.grant_sse2_class();
                self.enable_sse = true;
            }
            "-msse" => {
                self.no_sse = false;
                self.sse_explicitly_disabled = false;
                self.general_regs_only = false;
                if !self.sse2_explicitly_disabled {
                    self.no_sse2 = false;
                }
                self.enable_sse = true;
            }
            _ => return false,
        }
        true
    }

    /// Enter the ordered general-register-only state. This is shared by the
    /// immediate parse and post-profile replay so flag order cannot drift.
    fn deny_general_registers(&mut self) {
        self.general_regs_only = true;
        self.no_sse = true;
        self.no_sse2 = true;
        self.sse_explicitly_disabled = true;
        self.sse2_explicitly_disabled = true;
        self.avx_explicitly_disabled = true;
        self.fma_explicitly_disabled = true;
        self.sse41_explicitly_disabled = true;
        self.enable_sse2 = false;
        self.enable_sse3 = false;
        self.enable_ssse3 = false;
        self.enable_sse4_1 = false;
        self.enable_sse4_2 = false;
        self.enable_avx = false;
        self.enable_avx2 = false;
    }

    /// Explicit SSE2-or-higher grant: lift every SIMD denial, including a
    /// general-register restriction. CPU profiles deliberately do not use
    /// this helper; explicit denials stay sticky across `-march=`.
    fn grant_sse2_class(&mut self) {
        self.no_sse = false;
        self.no_sse2 = false;
        self.sse_explicitly_disabled = false;
        self.sse2_explicitly_disabled = false;
        self.general_regs_only = false;
        // Every SSE2-class grant lights the SSE2 level signal so the
        // resolved i686 levels keep it even over a sticky pre-SSE `-march=`
        // veto (GCC 16.2: `-m32 -march=i686 -msse2` keeps `__SSE2__`).
        self.enable_sse2 = true;
    }

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
    /// The AVX2 implication class (GCC 16.2 semantics, measured): `-mavx2`
    /// and every flag that implies it (AVX-VNNI and its int8/int16 forms,
    /// and all AVX-512 subsets) grant AVX2 with AVX, SSE4.2 and POPCNT.
    /// Explicit `-mno-sse` is overridden, as GCC does.
    /// The AVX class (`-mavx`): clears the xmm/AVX/SSE4.x denials and grants
    /// the SSE3..SSE4.2 ladder it contains, plus POPCNT (GCC defines
    /// `__POPCNT__` for `-mavx`). Shared by every flag that implies AVX.
    fn grant_avx_class(&mut self) {
        self.grant_sse2_class();
        self.sse41_explicitly_disabled = false;
        self.avx_explicitly_disabled = false;
        self.enable_avx = true;
        self.enable_sse4_2 = true;
        self.enable_sse4_1 = true;
        self.enable_ssse3 = true;
        self.enable_sse3 = true;
        // AVX ⊃ SSE4.2 ⇒ POPCNT (GCC, measured on gcc 14.2:
        // `-mavx -march=x86-64` defines `__POPCNT__`).
        self.enable_popcnt = true;
    }

    fn grant_avx2_class(&mut self) {
        // Explicit ISA enable wins over a prior `-mno-sse` (GCC).
        self.grant_sse2_class();
        self.sse41_explicitly_disabled = false;
        self.avx_explicitly_disabled = false;
        self.avx2_explicitly_disabled = false;
        self.enable_x86_avx2_profile();
        // SSE4.2 => POPCNT (GCC: the Nehalem bundle; `-mavx2
        // -march=x86-64` defines `__POPCNT__`, measured on gcc 14.2). AVX2
        // contains AVX contains SSE4.2, so the implied integer feature rides
        // with the explicit flag.
        self.enable_popcnt = true;
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
        self.enable_cx16 = true;
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
        // CMPXCHG16B is part of the x86-64-v2 contract; integer ISA, so it
        // stays legal under `-mno-sse` like POPCNT.
        self.enable_cx16 = true;
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

        // i386 has a real SSE-only state: unlike x86-64's existing backend,
        // `-mno-sse2` need not disable its baseline SSE XMM argument/return
        // carriers. Only an explicit all-SSE denial (or general-regs-only)
        // closes that register file; `-msse`/`-msse2` can subsequently lift it.
        if self.target == Target::I686 && self.no_sse2 {
            self.no_sse = self.sse_explicitly_disabled || self.general_regs_only;
        }

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
        // Distinct unrecognized spellings already diagnosed, so a flag passed
        // to every translation unit of a build reports once (see the
        // unknown-option arm). Also the `LCCC_STRICT_OPTIONS` escalation point.
        // `LCCC_STRICT_OPTIONS` escalates every accepted-but-unimplemented
        // spelling to an error.  Read once: the policy is per process, and a
        // build that cannot see one value for the whole command line is worse
        // than either answer.
        let strict_options = std::env::var_os("LCCC_STRICT_OPTIONS").is_some();
        // With the optional gcc_m16 delegate, GCC—not LCCC—owns target-affecting
        // options. Preserve its full TLS-model surface instead of rejecting an
        // option that will never reach LCCC codegen. Detect this from the whole
        // command line so option order cannot change the answer.
        let gcc_m16_passthrough = cfg!(feature = "gcc_m16") && args.iter().any(|arg| arg == "-m16");
        let mut unknown_options: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        // The last `-fprofile-dir=`, which a bare profile mode switch uses
        // wherever it appears relative to the switch.
        let mut profile_dir: Option<String> = None;
        let mut i = 0;
        while i < args.len() {
            // x86 ISA flags are routed through the shared applicator FIRST:
            // the same code path serves the `-march=` replace-and-replay
            // contract (an explicit flag's effect must be reproducible on
            // top of ANY later arch set), and recording here — the single
            // call site — keeps the replay log and the immediate effect
            // from ever drifting apart.
            if args[i].starts_with("-m") && self.apply_x86_isa_mflag(&args[i]) {
                self.x86_explicit_isa_args.push(args[i].clone());
                i += 1;
                continue;
            }
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
                // -fsyntax-only: check the translation unit (preprocess +
                // parse + sema) and stop — no codegen, no assembly, no
                // link, no output file, for ANY input kind (including
                // stdin). Previously this flag fell through to the unknown
                // argument handler and was silently dropped, so the driver
                // ran the Full pipeline and failed linking the empty TU's
                // missing `main` (WO-4, red-team audit 2026-09-18; the
                // i686 header gate had to probe with `-E` instead).
                "-fsyntax-only" => self.mode = CompileMode::SyntaxOnly,
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

                // Debug info — GCC flag semantics, honored exactly:
                //   -g / -ggdb / -g<N> / -ggdb<N> / -gdwarf-<N>  enable debug
                //     info (level and version selectors all mean "emit debug").
                //   -g0 / -ggdb0                                 explicitly
                //     DISABLE debug info (the old `starts_with("-g")` blanket
                //     inverted GCC's meaning of -g0).
                //   -gno-*                                       modifier forms
                //     that switch a debug FEATURE off; they must never turn
                //     debug info ON for a TU compiled without -g.
                //   -gsplit-dwarf: split DWARF (.dwo side files) is not
                //     implemented — warn and continue with non-split debug
                //     info rather than silently ignoring the request.
                //   -gz / -gz=<type>: compressed debug sections are not
                //     implemented — warn; the flag only selects compression,
                //     so it changes nothing about whether debug info is built.
                // Unknown `-g*` selectors keep the historical blanket-enable:
                // a future spelling must not silently turn debug OFF.
                "-g0" | "-ggdb0" => self.debug_info = false,
                "-g" | "-ggdb" => self.debug_info = true,
                "-gsplit-dwarf" => {
                    self.debug_info = true;
                    eprintln!(
                        "lccc: warning: -gsplit-dwarf is not supported; \
                         emitting non-split debug info"
                    );
                }
                "-gz" => {
                    eprintln!(
                        "lccc: warning: compressed debug sections (-gz) are not \
                         supported; emitting uncompressed debug info"
                    );
                    unknown_option_tolerated("-gz", strict_options)?;
                }
                arg if arg.starts_with("-gz=") => {
                    // `-gz=<method>` only selects a compression method, so it
                    // must never flip debug info on; but the method grammar is
                    // closed (GNU rejects `-gz=bogus`, measured on GCC
                    // 14.2/16.2), so an unknown method is an error, not a
                    // warning.
                    if !gnu_debug_selector_known(&arg[2..]) {
                        unknown_option(arg, strict_options)?;
                    }
                    eprintln!(
                        "lccc: warning: compressed debug sections ({}) are not \
                         supported; emitting uncompressed debug info",
                        arg
                    );
                    unknown_option_tolerated(arg, strict_options)?;
                }
                // A `-gno-<feature>` modifier asks for a feature to be OFF.
                // LCCC implements none of the features these modifiers name,
                // so the requested state is the actual state: the request is
                // satisfied by construction and no diagnostic is owed.  An
                // unknown modifier is a GNU error, though (`-gno-bogus` exits
                // 1 on GCC 14.2 and 16.2, measured), and accepting it here was
                // the other half of the old `-g*` blanket.
                arg if arg.starts_with("-gno-") => {
                    if !gnu_debug_feature_off_known(&arg[5..]) {
                        unknown_option(arg, strict_options)?;
                    }
                }
                arg if arg.len() > 5
                    && arg.starts_with("-ggdb")
                    && arg[5..].bytes().all(|b| b.is_ascii_digit()) =>
                {
                    // GCC accepts -ggdb1..-ggdb3; -ggdb4 and above are errors
                    // (measured on 14.2, and 16.2 rejects -ggdb9).
                    if !matches!(&arg[5..], "1" | "2" | "3") {
                        unknown_option(arg, strict_options)?;
                    }
                    self.debug_info = true
                }
                arg if arg.len() > 2
                    && arg.starts_with("-g")
                    && arg[2..].bytes().all(|b| b.is_ascii_digit()) =>
                {
                    // GCC accepts -g1..-g3 (-g4 is an error, measured); -g0 is
                    // handled above as an explicit OFF.
                    if !matches!(&arg[2..], "1" | "2" | "3") {
                        unknown_option(arg, strict_options)?;
                    }
                    self.debug_info = true
                }
                // Any other `-g<selector>`.  Two outcomes, both measured:
                //
                //  * GNU accepts the selector (a format/modifier spelling LCCC
                //    has no implementation for) -> keep the historical
                //    semantic, a selector must never silently turn debug OFF,
                //    and diagnose that nothing acts on it.
                //  * GNU rejects it (`-gbogus`, `-gdwarf-9`, `-glto`) -> error,
                //    matching GCC 14.2/16.2 and Clang 23.1/ICX; the old blanket
                //    accepted all of these and made the capability probe
                //    report them as implemented.
                arg if arg.starts_with("-g") => {
                    if !gnu_debug_selector_known(&arg[2..]) {
                        unknown_option(arg, strict_options)?;
                    }
                    self.debug_info = true;
                    unknown_option_tolerated(arg, strict_options)?;
                }

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

                // Preprocessor pass-through: -Wp,-MMD,path / -Wp,-MD,path /
                // -Wp,-MP.  The kernel build compiles with -Wp,-MMD,$@; the
                // system-header verdict MUST follow the flag (MMD filters,
                // MD keeps), exactly like the bare spellings above.
                arg if arg.starts_with("-Wp,") => {
                    let flags: Vec<&str> = arg[4..].splitn(2, ',').collect();
                    if flags.len() == 2 && (flags[0] == "-MMD" || flags[0] == "-MD") {
                        self.dep_file = Some(flags[1].to_string());
                        self.dep_exclude_system = flags[0] == "-MMD";
                    } else if flags.len() == 1 && flags[0] == "-MP" {
                        self.dep_phony = true;
                    }
                }

                // Warning flags.
                //
                // `process_flag` reports whether it recognised the name; an
                // unknown one must not vanish.  The measured contract
                // (2026-10-04, GCC 14.2 in-tree plus the pinned oracles
                // GCC 16.2 / Clang 23.1 / ICC 2021.10 / ICX):
                //
                //   -W<name>          gcc: rc 1   clang/icc/icx: accept
                //   -Werror=<name>    gcc: rc 1   clang/icc/icx: accept
                //   -Wno-error=<name> gcc: rc 1   clang/icc/icx: accept
                //   -Wno-<name>       all four: accept, silently
                //
                // LCCC implements SIX warning names; GNU exposes 409
                // (`gcc -Q --help=warnings | grep -c '^  -W'`, measured).  A
                // driver with six names of a 409-name namespace cannot decide
                // whether a name it does not implement is a typo or a warning
                // added by a newer GNU -- so it must not guess in the
                // direction that stops a build that GNU builds, and it must
                // not guess "supported" either.  Both are avoided by
                // diagnosing the name (which a `-Wno-` request does not need:
                // LCCC emits no such warning, so "off" is already the state)
                // and making it fatal under `LCCC_STRICT_OPTIONS=1`, which is
                // what the capability probe exports.  The consequence of
                // tolerating a name is a missing diagnostic, never different
                // code -- which is why this namespace, unlike `-f`/`--`/`-g`,
                // is not fail-closed by default.
                // `-w`: inhibit all warning messages.  Measured against
                // GCC 16.2 (2026-10-17): dominant regardless of position
                // (`-w -Wall` and `-Wall -w` are both silent, `#warning`
                // included), hence a sticky latch in `WarningConfig`.  The
                // driver's own option-contract diagnostics (tolerated
                // unknown options, LCCC_STRICT_OPTIONS) are NOT user-code
                // warnings and stay audible.
                "-w" => self.warning_config.set_inhibit_all(),

                arg if arg.starts_with("-W") => {
                    let flag = &arg[2..];
                    // Every flag reaches the warning configuration first
                    // (`-Wno-return-type`, `-Wno-error=cpp` are real
                    // requests: short-circuiting on the `no-` prefix once
                    // dropped all of them). An UNKNOWN `-Wno-<name>` is then
                    // honoured by construction (there is no such warning, so
                    // "off" is the state) and gets no diagnostic, exactly as
                    // the GNU convention has it; every other unknown name is
                    // diagnosed by name.
                    if !flag.is_empty()
                        && !self.warning_config.process_flag(flag)
                        && !flag.starts_with("no-")
                    {
                        unknown_option_tolerated(arg, strict_options)?;
                    }
                }

                // Preprocessor defines
                "-D" => {
                    i += 1;
                    if i < args.len() {
                        self.add_define(&args[i], strict_options)?;
                    } else {
                        return Err("-D requires an argument".to_string());
                    }
                }
                arg if arg.starts_with("-D") => self.add_define(&arg[2..], strict_options)?,

                // `--sysroot=<dir>` / `-isysroot <dir>`: implemented by the
                // link layer (`backend::common` resolves the CRT objects, the
                // default library search and `-print-libgcc-file-name` against
                // it) and by the preprocessor's system include search.  The
                // operand must be consumed here so it is not mistaken for an
                // input file; `raw_args` carries the spelling to the layers
                // that act on it.
                arg if arg.starts_with("--sysroot=") || arg.starts_with("-isysroot=") => {}
                "--sysroot" | "-isysroot" => {
                    i += 1;
                    if i >= args.len() {
                        return Err(format!("{} requires a directory argument", args[i - 1]));
                    }
                }

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
                // GCC treats the negative spellings as the opposite flag
                // (measured on GCC 16.2: `-fno-signed-char` == `-funsigned-char`).
                "-funsigned-char" | "-fno-signed-char" => {
                    crate::common::types::set_char_unsigned(true);
                }
                "-fsigned-char" | "-fno-unsigned-char" => {
                    crate::common::types::set_char_unsigned(false);
                }
                // `-fshort-wchar`: 16-bit unsigned `wchar_t` (GCC's
                // `__WCHAR_TYPE__ = short unsigned int`). Recorded here, before
                // any input file is parsed, so the parser, sema and lowering all
                // see one consistent `wchar_t`. Required by the Linux kernel,
                // which builds with this flag.
                arg if arg == "-fshort-wchar" => {
                    crate::common::types::set_short_wchar(true);
                }
                arg if arg == "-fno-short-wchar" => {
                    crate::common::types::set_short_wchar(false);
                }
                // S13: function entry/exit instrumentation via the
                // __cyg_profile_func_enter/exit hooks (GCC contract).
                arg if arg == "-finstrument-functions" => {
                    self.instrument_functions = true;
                }
                arg if arg == "-finstrument-functions-exclude-file-list" => {}
                arg if arg == "-finstrument-functions-exclude-function-list" => {}
                // `-ansi` is `-std=c90` in GCC's driver (measured: `gcc
                // -ansi -dM -E` reports `__STDC_VERSION__` absent, exactly
                // like `-std=c90`, and defines `__STRICT_ANSI__`).  Ignoring
                // it would compile C90 sources as C17, which is the one thing
                // the dialect policy never does silently.
                // NOTE: these arms must not `return` — the parse loop still
                // has the input files and every other option to walk.
                "-ansi" => self.apply_std_dialect("c90")?,
                arg if arg.starts_with("-std=") => self.apply_std_dialect(&arg[5..])?,

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
                // AVX10 is a hard error (not silently ignored): requesting
                // code generation the backend cannot produce must fail.
                "-mavx10.1" | "-mavx10.1-256" | "-mavx10.1-512" | "-mavx10.2" | "-mavx10.2-256"
                | "-mavx10.2-512" => {
                    return Err("AVX10 code generation is not implemented".to_string());
                }
                "-mxsave" | "-mxsaveopt" | "-mxsavec" | "-mno-xsave" | "-mno-xsaveopt"
                | "-mno-xsavec" => {}
                "-mmmx" => {}
                "-m3dnow" => return Err("3DNow! is unsupported".to_string()),
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
                        // A later `-march=` REPLACES the arch-implied set
                        // (GCC: `-march=x86-64-v3 -march=x86-64` leaves
                        // `__BMI__`/`__MOVBE__`/`__AVX__` absent — measured,
                        // gcc 14.2 `-dM -E`); the recorded explicit flags
                        // are replayed on top of the new profile below.
                        self.reset_x86_arch_implied_isa();
                    }
                    match self.target {
                        Target::Riscv64 => self.riscv_march = Some(march.to_string()),
                        Target::X86_64 | Target::I686 => {
                            // The baseline SSE/SSE2/MMX grant follows the
                            // FINAL `-march=` (last wins, like the profile):
                            // every accepted spelling resets the pre-SSE veto,
                            // and the legacy 32-bit baselines re-set it.
                            self.march_pre_sse = false;
                            match march {
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
                                    // Pre-SSE baseline: vetoes the default
                                    // x86-64-baseline SSE/SSE2/MMX grant on i686
                                    // (GCC 16.2 `-m32 -march=i686` measures
                                    // `-msse`/`-msse2` [disabled], no `__MMX__`);
                                    // explicit `-msse`/`-msse2` still override at
                                    // resolve time, and MMX follows the arch (the
                                    // kernel's `-march=i386 -m16` real-mode code
                                    // gets neither, as measured).
                                    self.march_pre_sse = true;
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
                                | "gracemont" | "alderlake-n" => {
                                    self.enable_x86_alderlake_profile()
                                }
                                // Sierra Forest / Grand Ridge (Crestmont E-core): Alder
                                // Lake ISA + AVX-IFMA, AVX-NE-CONVERT, AVX-VNNI-INT8,
                                // CMPCCXADD — the same additions Arrow Lake carries
                                // (GCC `PTA_SIERRAFOREST`, LLVM `SRFFeatures`).
                                "arrowlake" | "sierraforest" | "grandridge" => {
                                    self.enable_x86_arrowlake_profile()
                                }
                                // Clearwater Forest (Darkmont) adds AVX-VNNI-INT16
                                // like Arrow Lake-S / Lunar Lake.
                                "arrowlake-s" | "lunarlake" | "wildcatlake"
                                | "clearwaterforest" => {
                                    self.enable_x86_arrowlake_profile();
                                    self.enable_avxvnniint16 = true;
                                }
                                "x86-64-v4" => self.enable_x86_avx512_profile(),
                                "skylake-avx512" => self.enable_x86_avx512_cpu_profile(),
                                "cascadelake" => self.enable_x86_cascadelake_profile(),
                                "cooperlake" => self.enable_x86_cooperlake_profile(),
                                "icelake-client" | "icelake-server" | "tigerlake"
                                | "rocketlake" => self.enable_x86_icelake_profile(),
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
                                    ));
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
                                        if std::arch::is_x86_feature_detected!("cmpxchg16b") {
                                            self.enable_cx16 = true;
                                        }
                                        // Integer-side hint ISA (no xmm state),
                                        // so it is not gated on `-mno-sse`.
                                        if crate::backend::x86::cpu_model::X86Cpu::host_has_prfchw()
                                        {
                                            self.enable_prfchw = true;
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
                                                // VL is its own CPUID bit, not implied by F:
                                                // Xeon Phi (KNL: F+CD+ER+PF, no VL) plus
                                                // several early-SP parts have F without
                                                // VL, and the xmm/ymm EVEX forms (e.g.
                                                // `vprold`) are #UD there.  The profile
                                                // grants VL unconditionally; clear it
                                                // unless the host really has it (GCC's
                                                // native probe reads the VL bit too).
                                                if !std::arch::is_x86_feature_detected!("avx512vl")
                                                {
                                                    self.enable_avx512vl = false;
                                                }
                                            }
                                        }
                                    }
                                    #[cfg(not(target_arch = "x86_64"))]
                                    {
                                        return Err("-march=native requires an x86-64 host; pass an explicit profile".to_string());
                                    }
                                }
                                _ => return Err(format!("unsupported x86 -march={}", march)),
                            }
                        }
                        _ => {
                            return Err(format!(
                                "-march={} is not implemented for target {}",
                                march,
                                self.target.triple()
                            ));
                        }
                    }
                    // Explicit ISA decisions land on top of the FINAL arch
                    // set, in command-line order (GCC: explicit flags
                    // override the arch in BOTH directions; among
                    // explicits, later wins).  Measured on gcc 14.2:
                    // `-mno-bmi -march=x86-64-v3` → `__BMI__` absent;
                    // `-mbmi -march=x86-64` → `__BMI__` defined.
                    if matches!(self.target, Target::X86_64 | Target::I686) {
                        if x86_march_implies_prfchw(march) {
                            self.enable_prfchw = true;
                        }
                        self.replay_x86_explicit_isa_flags();
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
                            ));
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
                            arg, bytes
                        ));
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
                // The dynamic TLS models are emitted in ONE dialect per target:
                // TLS descriptors on x86 (`gnu2`, preserves every register but
                // the result, so the access is not a call to the register
                // allocator) and AArch64 (`desc`, GCC's default there), the
                // traditional `__tls_get_addr` sequence on RISC-V (`trad`,
                // GCC's default there). Validated after the loop because `-m32`
                // may still change the target. Must precede the `-m` catch-all
                // below (guarded arms are not checked for reachability).
                arg if arg.starts_with("-mtls-dialect=") => {
                    self.tls_dialect = Some(arg["-mtls-dialect=".len()..].to_string());
                }
                arg if arg.starts_with("-mstack-protector-guard") => {
                    return Err(format!(
                        "{}: LCCC does not implement stack-protector canaries; \
                         build with -fno-stack-protector",
                        arg
                    ));
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
                    return Err(format!(
                        "unsupported machine option {}; LCCC refuses to silently ignore target-affecting -m flags",
                        arg
                    ));
                }

                // PGO flags (instrumented profiling), with GCC 16.2's measured
                // path rules: the later of `-fprofile-dir=D` and an explicit
                // `-fprofile-generate=P`/`-fprofile-use=P` decides the
                // directory, a bare mode switch uses the `-fprofile-dir` given
                // before OR after it, and an empty path is an error.
                "-fprofile-generate" | "-fprofile-arcs" | "-ftest-coverage" => {
                    self.pgo_generate =
                        Some(profile_dir.clone().unwrap_or_else(|| ".".to_string()));
                }
                arg if arg.starts_with("-fprofile-generate=") => {
                    self.pgo_generate = Some(profile_path_arg(arg, "-fprofile-generate=")?);
                }
                "-fprofile-use" => {
                    self.pgo_use = Some(profile_dir.clone().unwrap_or_else(|| ".".to_string()));
                }
                arg if arg.starts_with("-fprofile-use=") => {
                    self.pgo_use = Some(profile_path_arg(arg, "-fprofile-use=")?);
                }
                arg if arg.starts_with("-fprofile-update=") => {
                    // GCC 7+/LLVM 17+ semantics: single (default, non-atomic),
                    // atomic (lock-prefixed), prefer-atomic (atomic when the
                    // target supports it — x86-64 always does).
                    self.pgo_update = Some(match profile_path_arg(arg, "-fprofile-update=")? {
                        m if m == "atomic" || m == "prefer-atomic" => "atomic".to_string(),
                        m if m == "single" => m,
                        m => {
                            return Err(format!(
                                "unknown profile update method '{m}' \
                                 (-fprofile-update={m}; valid arguments are: \
                                 atomic prefer-atomic single)"
                            ));
                        }
                    });
                }
                "-fbranch-probabilities" => {
                    self.pgo_use =
                        Some(self.pgo_use.clone().unwrap_or_else(|| {
                            profile_dir.clone().unwrap_or_else(|| ".".to_string())
                        }))
                }
                arg if arg.starts_with("-fprofile-dir=") => {
                    let path = profile_path_arg(arg, "-fprofile-dir=")?;
                    if self.pgo_generate.is_some() {
                        self.pgo_generate = Some(path.clone());
                    }
                    if self.pgo_use.is_some() {
                        self.pgo_use = Some(path.clone());
                    }
                    profile_dir = Some(path);
                }
                "-fauto-profile" => self.pgo_use = Some(".".to_string()),
                arg if arg.starts_with("-fauto-profile=") => {
                    self.pgo_use = Some(profile_path_arg(arg, "-fauto-profile=")?);
                }
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
                // GCC's `flag_tls_default`: a FLOOR every TLS symbol's model is
                // raised to (common::tls::select_model); per-declaration
                // `tls_model` attributes raise it further. The last one wins.
                // Under `-m16` GCC delegation the option passes through to GCC
                // unvalidated, as every other option does.
                arg if arg.starts_with("-ftls-model=") => {
                    let model = &arg["-ftls-model=".len()..];
                    match crate::common::tls::TlsModel::parse(model) {
                        Some(m) => self.tls_model_floor = Some(m),
                        None if gcc_m16_passthrough => {}
                        // GCC 16.2's wording, measured (error + note).
                        None if model.is_empty() => {
                            return Err("missing argument to '-ftls-model='".to_string());
                        }
                        None => {
                            return Err(format!(
                                "unknown TLS model '{model}'; valid arguments to '-ftls-model=' \
                                 are: global-dynamic initial-exec local-dynamic local-exec"
                            ));
                        }
                    }
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
                    let value = &arg["-fpatchable-function-entry=".len()..];
                    let parts: Vec<&str> = value.split(',').collect();
                    let invalid = || {
                        format!(
                            "invalid value '{}' for '{}'; expected N[,M] with 0 <= M <= N",
                            value, arg
                        )
                    };
                    if parts.is_empty() || parts.len() > 2 {
                        return Err(invalid());
                    }
                    let total = parts[0].parse::<u32>().map_err(|_| invalid())?;
                    let before = match parts.get(1) {
                        Some(part) => part.parse::<u32>().map_err(|_| invalid())?,
                        None => 0,
                    };
                    if before > total {
                        return Err(invalid());
                    }
                    self.patchable_function_entry = Some((total, before));
                }
                "-fomit-frame-pointer" => self.omit_frame_pointer = true,
                "-fno-omit-frame-pointer" => self.omit_frame_pointer = false,
                // ── Code alignment control ─────────────────────────────────
                // GCC grammar: -falign-loops[=N[:M]] / -fno-align-loops, with
                // N a power-of-two byte alignment and M an optional max-skip
                // (the third .p2align operand). A bare -falign-loops selects
                // the -O-level default rather than a fixed value.
                "-falign-loops" => self.align_loops = AlignControl::Auto,
                "-fno-align-loops" => self.align_loops = AlignControl::Off,
                "-falign-jumps" => self.align_jumps = AlignControl::Auto,
                "-fno-align-jumps" => self.align_jumps = AlignControl::Off,
                "-falign-functions" => self.align_functions = AlignControl::Auto,
                "-fno-align-functions" => self.align_functions = AlignControl::Off,
                arg if arg.starts_with("-falign-loops=")
                    || arg.starts_with("-falign-jumps=")
                    || arg.starts_with("-falign-functions=") =>
                {
                    let (kind, value) = arg.split_once('=').unwrap();
                    match Self::parse_align_spec(value) {
                        Some(control) => match kind {
                            "-falign-loops" => self.align_loops = control,
                            "-falign-jumps" => self.align_jumps = control,
                            _ => self.align_functions = control,
                        },
                        None => {
                            return Err(format!(
                                "invalid argument to {kind}: alignment must be a power of 2 \
                                 optionally followed by ':max-skip' (e.g. 32:8), got {value}"
                            ));
                        }
                    }
                }
                "-fno-asynchronous-unwind-tables" | "-fno-unwind-tables" => {
                    self.no_unwind_tables = true
                }
                "-fasynchronous-unwind-tables" | "-funwind-tables" => self.no_unwind_tables = false,
                "-fno-jump-tables" => self.no_jump_tables = true,
                // (-ffreestanding/-fhosted/-fbuiltin/-fno-builtin are
                // handled by the A13 arms above; per-name `-fno-builtin-<fn>`
                // withdrawals are supported there with GCC's stickiness.)
                "-fno-tree-vectorize" => self.disable_tree_vectorize = true,
                "-ftree-vectorize" => self.disable_tree_vectorize = false,
                "-fipa-pta" => self.ipa_pta = true,
                "-fno-ipa-pta" => self.ipa_pta = false,
                "-flive-range-shrinkage" => self.live_range_shrinkage = true,
                "-fno-live-range-shrinkage" => self.live_range_shrinkage = false,
                arg if matches!(
                    arg,
                    "-fno-tree-loop-vectorize"
                        | "-fno-tree-slp-vectorize"
                        | "-ftree-loop-vectorize"
                        | "-ftree-slp-vectorize"
                ) =>
                {
                    return Err(format!("per-vectorizer control is unsupported: {}", arg));
                }
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
                "-frounding-math" => self.fp_rounding_math = true,
                "-fno-rounding-math" => self.fp_rounding_math = false,
                "-ftrapping-math" => self.fp_trapping_math = true,
                "-fno-trapping-math" => self.fp_trapping_math = false,
                "-fwrapv" => self.wrap_signed_overflow = true,
                "-fno-wrapv" => self.wrap_signed_overflow = false,
                // Builtin status and hosted-ness, with GCC 16.2's exact
                // semantics (measured on the oracle; pinned by
                // `builtin_flags_match_gcc`): the blanket flags are
                // last-wins, `-fno-builtin-<fn>` is sticky, and the hosted
                // flags also set the blanket builtin state while
                // `-fbuiltin` leaves hosted-ness alone.
                "-ffreestanding" | "-fno-hosted" => {
                    self.freestanding = true;
                    self.builtin_policy.withdraw_all();
                }
                "-fhosted" | "-fno-freestanding" => {
                    self.freestanding = false;
                    self.builtin_policy.restore_all();
                }
                "-fbuiltin" => self.builtin_policy.restore_all(),
                "-fno-builtin" => self.builtin_policy.withdraw_all(),
                arg if arg.starts_with("-fno-builtin-") => {
                    self.builtin_policy
                        .withdraw_name(&arg["-fno-builtin-".len()..]);
                }
                "-ffast-math" => {
                    self.fp_trapping_math = false;
                    self.fp_rounding_math = false;
                    self.fast_math = true;
                    self.fp_reassoc = true;
                    self.math_errno = false;
                    // GCC: fast-math implies contract=fast only when no
                    // explicit -ffp-contract was given (opts_set semantics).
                    if !self.fp_contract_explicit {
                        self.fp_contract = FpContract::Fast;
                    }
                }
                "-fno-fast-math" => {
                    self.fast_math = false;
                    self.fp_reassoc = false;
                    self.math_errno = true;
                    // Restore the language default. In GCC, -ffp-contract is
                    // orthogonal to -fno-fast-math: the C default (fast)
                    // stands. Explicit -ffp-contract={on,off,fast} wins.
                    if !self.fp_contract_explicit {
                        self.fp_contract = FpContract::c_language_default();
                    }
                }
                "-fmath-errno" => self.math_errno = true,
                "-fno-math-errno" => self.math_errno = false,
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
                    self.color_mode = ColorMode::from_flag(value).ok_or_else(|| {
                        format!(
                            "invalid value '{}' for '{}'; expected auto, always, or never",
                            value, arg
                        )
                    })?;
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
                // Data-model and ABI requests.  A *suppression* request can be
                // satisfied by not doing the thing; these cannot: they ask for
                // a different `char` signedness, enum width, wchar_t width or
                // struct packing, so ignoring one silently produces objects
                // whose layout disagrees with the libraries they are linked
                // against.  `-fsigned-char` is not here: it names the x86-64
                // SysV default, which is what LCCC already emits.
                // `-funsigned-char` is NOT in this arm: LCCC implements it
                // (see the dedicated arm above; `char_is_unsigned()` is
                // honoured by the parser, the preprocessor's predefined
                // macros and lowering), so it is a supported request, not a
                // data-model refusal.
                // `-fshort-wchar` is NOT in this arm either: LCCC implements
                // it (`set_short_wchar`, dedicated arm above).
                a @ ("-fshort-enums" | "-fpack-struct") => {
                    return Err(format!(
                        "{}: LCCC does not implement this data-model request, and \
                         ignoring it would silently produce objects whose layout or \
                         char signedness disagrees with the code it links against",
                        a
                    ));
                }
                arg if arg.starts_with("-fpack-struct=") => {
                    return Err(format!(
                        "{}: LCCC does not implement packed struct layouts; define the \
                         struct with __attribute__((packed)) instead",
                        arg
                    ));
                }
                // NOTE: there used to be a blanket `arg if arg.starts_with("-f") => {}`
                // here. It made every unimplemented `-f` option -- including
                // `-ftrapv` and `-fsanitize=*`, whose explicit refusal arms
                // sat below it and were therefore unreachable dead code --
                // disappear without a diagnostic. Unimplemented `-f`
                // spellings are now classified by `option_table`: a deliberate
                // no-op carries its reason there, everything else refuses.
                // Do not reintroduce the blanket no-op; add a specific,
                // commented table row for a flag LCCC deliberately treats as
                // inert. Implemented `-f` flags keep their specific arms
                // ABOVE this fall-through.
                arg if arg.starts_with("-f") => {
                    use super::option_table::{Class, lookup};
                    match lookup(arg) {
                        // Deliberate no-op: the table row carries the reason.
                        Some(c) if c.class == Class::BenignNoop => {}
                        // Classified refusal: the request would change program
                        // meaning (LTO, sanitizers, ABI, ...), so a capability
                        // probe must read "unsupported" and fall back.
                        Some(c) => return Err(format!("{}: {}", arg, c.rationale)),
                        // A table miss is a spelling NEWER than this tree
                        // knows.  Give it the generic tier treatment
                        // (tolerated diagnostic, fatal under
                        // LCCC_STRICT_OPTIONS=1) rather than a hard refusal:
                        // a newer GNU's inert optimisation preference must not
                        // break a real build, and strict mode is what turns
                        // the tolerance into the answer a probe needs.
                        None => {
                            if unknown_options.insert(arg.to_string()) {
                                unknown_option_tolerated(arg, strict_options)?;
                            }
                        }
                    }
                }

                // Linker flags
                "-static" => self.static_link = true,
                "-shared" => self.shared_lib = true,
                "-r" | "-relocatable" => self.relocatable = true,
                // Codegen already defaults to PIC, but the *link* only becomes
                // ET_DYN when the linker is told.  Record the flag positionally
                // so `parse_linker_args` sees it; dropping it silently produced
                // ET_EXEC at a fixed base, i.e. no ASLR on every binary.
                "-pie" | "--pie" | "--pic-executable" => {
                    self.linker_ordered_items.push("-pie".to_string());
                }
                "-no-pie" | "--no-pie" | "--no-pic-executable" => {
                    self.linker_ordered_items.push("-no-pie".to_string());
                }
                // The three GNU link-file switches are recorded here and
                // forwarded by `build_linker_args` to `backend::common`, which
                // acts on them.  (Recognising `-nostartfiles`/`-nodefaultlibs`
                // without forwarding them made both silent no-ops: crt1/crti
                // were still linked, so a freestanding `_start` collided with
                // crt1's and a `-shared -nostartfiles` object got crtbeginS.)
                //
                // Semantics, measured on GCC 14.2 and 16.2 and honoured as measured:
                //   -nostartfiles  -> no crt1/crti/crtbegin/crtend/crtn, libc kept
                //   -nodefaultlibs -> crt files kept, libc/libm AND libgcc dropped
                //   -nostdlib      -> crt files and all default libraries dropped
                "-nostdlib" => self.nostdlib = true,
                "-nostdinc" => self.nostdinc = true,
                "-nostartfiles" => self.nostartfiles = true,
                "-nodefaultlibs" => self.nodefaultlibs = true,

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

                // Dependency generation flags — GCC semantics:
                //   -MD: deps including system-directory headers,
                //   -MMD: deps with system-directory headers filtered out
                //   (the filter is DIRECTORY-based, not bracket-based),
                //   -MP: emit phony rules so deleting a header doesn't
                //   break an unchanged make graph,
                //   -M / -MM: dependency-only mode (implies -E); -MM
                //   applies the same system-directory filter as -MMD.
                "-MD" => {
                    if self.dep_file.is_none() {
                        self.dep_file = Some(String::new());
                    }
                    self.dep_exclude_system = false;
                }
                "-MMD" => {
                    if self.dep_file.is_none() {
                        self.dep_file = Some(String::new());
                    }
                    self.dep_exclude_system = true;
                }
                "-MP" => self.dep_phony = true,
                "-M" => {
                    self.dep_only = true;
                    self.mode = CompileMode::PreprocessOnly;
                    self.dep_exclude_system = false;
                }
                "-MM" => {
                    self.dep_only = true;
                    self.mode = CompileMode::PreprocessOnly;
                    self.dep_exclude_system = true;
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
                // GCC's parameter table is implementation-private and every
                // spelling a real build passes is a request to tune an
                // internal heuristic, not a semantic contract; lccc has no
                // table to check against, so the value is tolerated — but
                // not silently (gcc 16.2 errors on `--param bogus=1`, and a
                // probe must be able to see that lccc has no implementation
                // for the name).
                "--param" => {
                    // Skip the next argument (the parameter value)
                    i += 1;
                    unknown_option_tolerated("--param", strict_options)?;
                }
                arg if arg.starts_with("--param=") => {
                    // Single-argument form: --param=ssp-buffer-size=4
                    unknown_option_tolerated(arg, strict_options)?;
                }

                // Stdin input
                "-" => {
                    self.input_files.push("-".to_string());
                    self.explicit_language = explicit_language.clone();
                }

                // GCC hard-errors on a bare `-soname`/--soname (unrecognized
                // option); the spelling it understands is `-Wl,-soname,NAME`.
                // Swallowing it silently (the old behaviour) left the value
                // behind as a phantom positional input — later producing a
                // baffling "not a relocatable object" link diagnostic instead
                // of GCC's early, clear error.
                arg @ ("-soname" | "--soname") => {
                    return Err(format!(
                        "unrecognized command-line option '{}' (use -Wl,-soname,NAME)",
                        arg
                    ));
                }
                arg if arg.starts_with("-soname=") || arg.starts_with("--soname=") => {
                    return Err(format!(
                        "unrecognized command-line option '{}' (use -Wl,-soname,NAME)",
                        arg
                    ));
                }

                // Instrumentation and trap requests whose *absence* is
                // invisible in the produced binary.
                //
                // Same failure mode as `-fstack-protector` above, and the same
                // answer: refuse loudly instead of handing back a binary the
                // caller believes is instrumented. `-ftrapv` promises a
                // run-time trap on signed overflow; LCCC emits no such check,
                // so accepting it would silently give a *different execution*
                // than requested. `-fsanitize=*` promises a checked build; an
                // unchecked binary that the caller will not re-verify in the
                // field is the exact hazard this arm exists to prevent.
                //
                // `-fno-trapv` / `-fsanitize-recover=*` style *disable*
                // requests stay accepted (see the `-mno-` arm) because a
                // compiler that never emits the instrumentation already
                // complies with a request to leave it out.
                arg if arg == "-ftrapv" => {
                    return Err("-ftrapv: LCCC does not emit signed-overflow traps; \
                         build without it, or use -fwrapv to state the \
                         wrapping contract explicitly"
                        .to_string());
                }
                arg if arg == "-fsanitize" || arg.starts_with("-fsanitize=") => {
                    return Err(format!(
                        "{}: LCCC does not implement sanitizer instrumentation; \
                         a build must not be labelled sanitized when it is not",
                        arg
                    ));
                }

                // Unknown flags -- LCCC's standing doctrine is that no request
                // may be discarded silently (see the module header and the
                // `-fstack-protector` arm). This arm is the generic case.
                //
                // Why a warning and not GCC's error: build systems probe with
                // speculative flags (`cc-option`, `meson.get_compiler().
                // has_argument`, Kconfig) and *rely* on the compiler exiting
                // successfully. Making the generic case fatal would abort the
                // Linux kernel build at the first Kconfig probe. A visible,
                // specific diagnostic plus `LCCC_STRICT_OPTIONS=1` gives CI and
                // the corpus harness the fatal behaviour without regressing
                // those probes.
                //
                // Reported once per distinct spelling: a build that passes the
                // same unimplemented flag to 4 000 translation units must
                // produce one line, not 4 000.
                arg if arg.starts_with('-') => {
                    if unknown_options.insert(arg.to_string()) {
                        // The policy lives in a pure function so it is testable
                        // without mutating process-global environment state
                        // (which would race under the parallel test harness).
                        //
                        // Tolerated with a diagnostic, not fatal: measured, 16
                        // of 93 flag spellings that the kernel, glibc, zlib-ng,
                        // gzip and expat build systems pass unconditionally
                        // have no arm here (`-fno-strict-aliasing`, `-fwrapv`,
                        // `-fno-strict-overflow`, `-fno-plt`, `-flto`,
                        // `-funroll-loops`, `-Og`, `-ansi`, `-fno-ident`, ...).
                        // Failing the build on those would break real builds
                        // while the request itself is an optimisation,
                        // reporting or debug-info preference -- the classes
                        // whose being ignored cannot change the generated
                        // program's *meaning*.  The requests that would change
                        // it are refused above (stack protector, data model,
                        // ABI) and the dialect request is implemented.
                        unknown_option_tolerated(arg, strict_options)?;
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
        if let Some(dialect) = self.tls_dialect.as_deref() {
            check_tls_dialect(&self.target, dialect)?;
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
    pub fn add_define(&mut self, arg: &str, strict: bool) -> Result<(), String> {
        let (name, value) = match arg.find('=') {
            Some(eq_pos) => (&arg[..eq_pos], arg[eq_pos + 1..].to_string()),
            None => (arg, "1".to_string()),
        };
        // `_FORTIFY_SOURCE` is accepted but neutralised (the driver undefines
        // it unconditionally; glibc's fortify wrappers are not modelled).  A
        // hardening request that cannot be honoured must not disappear
        // silently, so it goes through the same visible-tolerance policy as
        // the other accepted-but-unimplemented spellings.
        if name == "_FORTIFY_SOURCE" {
            fortify_source_unsupported(strict)?;
        }
        self.defines.push(CliDefine {
            name: name.to_string(),
            value,
        });
        Ok(())
    }

    /// Apply a `-std=<dialect>` selection (`-ansi` is `-std=c90`, see below).
    ///
    /// The one request class that cannot be satisfied by ignoring it: a dialect
    /// selects what the source is allowed to mean, so compiling C90 source as
    /// C17 accepts `//` comments, declarations after statements and `inline`
    /// that C90 rejects — the silent mis-parse the dialect policy refuses
    /// everywhere else.  An unknown name is therefore an error with a
    /// suggestion, and every known name sets the macros the dialect implies.
    fn apply_std_dialect(&mut self, std_value: &str) -> Result<(), String> {
        // GNU dialects: gnu89, gnu99, gnu11, gnu17, gnu23, etc.
        // Strict ISO: c89, c99, c11, c17, c23, iso9899:*, etc.
        self.gnu_extensions = std_value.starts_with("gnu");
        if self.gnu89_inline_explicit.is_none() {
            self.gnu89_inline = matches!(
                std_value,
                "gnu89" | "c89" | "gnu90" | "c90" | "iso9899:1990" | "iso9899:199409"
            );
        }
        self.finish_std_dialect(std_value)
    }

    /// Set `__STDC_VERSION__` for a dialect name, refusing an unknown one.
    fn finish_std_dialect(&mut self, std_value: &str) -> Result<(), String> {
        let version = std_version_macro(std_value);
        if version.is_none() {
            return Err(match closest_std_dialect(std_value) {
                Some(s) => format!(
                    "unrecognized command-line option '-std={}'; did you mean '-std={}'?",
                    std_value, s
                ),
                None => format!("unrecognized command-line option '-std={}'", std_value),
            });
        }
        self.stdc_version = version;
        Ok(())
    }

    /// Add a -I include path from command line.
    pub fn add_include_path(&mut self, path: &str) {
        self.include_paths.push(path.to_string());
    }
}

/// Decide what to do about one unrecognized command-line option.
///
/// **Fatal by default, matching GCC** (`gcc -fbogus`, `gcc -Wbogus`,
/// `gcc -gbogus` and `gcc --param bogus=1` all exit 1 with
/// `error: unrecognized command-line option`; measured on GCC 14.2 in this
/// tree and GCC 16.2 / clang 23.1 / icc 2021.10 / icx on the pinned oracles).
///
/// Why not the old warn-and-continue default: a build system decides whether
/// a flag exists by *the exit status* (`cc-option`, Meson `has_argument`,
/// Kconfig).  Succeeding on a flag lccc does not implement turns "does the
/// compiler support this?" into a false yes, and the build then proceeds
/// under assumptions the compiler never agreed to — for hardening or
/// optimisation flags that is a silent misbuild, which is exactly what this
/// module's doctrine forbids.
///
/// `strict` is `LCCC_STRICT_OPTIONS` being set; it is accepted so the error
/// text can say why the failure happened in strict mode.  Kept free of
/// global state (no `env::var`, no I/O beyond the deliberate diagnostic) so
/// the policy can be pinned by tests and reviewed as one unit.
pub fn unknown_option(arg: &str, strict: bool) -> Result<(), String> {
    if strict {
        return Err(format!(
            "unrecognized command-line option '{}'; LCCC_STRICT_OPTIONS is set",
            arg
        ));
    }
    Err(format!(
        "unrecognized command-line option '{}' (LCCC has no implementation for it; \
         refusing to let a feature probe read this as support)",
        arg
    ))
}

/// Is `selector` (the text after `-g`) a debug selector GNU accepts?
///
/// Measured, not guessed: `gcc -Q --help=common` for the name set, plus one
/// compile per spelling to pin the accepted/rejected boundary on GCC 14.2 in
/// this tree and GCC 16.2 on the pinned oracle.  Accepted: `dwarf`,
/// `dwarf-2..dwarf-5` (`-gdwarf-1`, `-gdwarf-6` and `-gdwarf-9` are errors on
/// both), `dwarf32`/`dwarf64`, the format selectors (`stabs`, `stabs+`,
/// `xcoff`, `xcoff+`, `coff1..3`, `vms`, `codeview`, `btf`, `ctf`), the
/// content modifiers (`column-info`, `record-gcc-switches`, `pubnames`,
/// `gnu-pubnames`, `statement-frontiers`, `variable-location-views`,
/// `internal-reset-location-views`, `describe-dies`, `strict-dwarf`,
/// `as-loc-support`, `gas-locview-support`, `inline-points`, `toggle`) and
/// `z` / `z=<none|zlib|zlib-gnu|zlib-gabi|zstd>` (`-gz=bogus` is an error).
/// Rejected, and therefore errors here: `g4`, `ggdb9`, `gdwarf-9`, `glto`,
/// `gbogus`.
///
/// LCCC honours the selectors it implements in the literal arms above; every
/// selector this function accepts but does not implement is diagnosed once
/// per compilation and fatal under `LCCC_STRICT_OPTIONS=1`.
fn gnu_debug_selector_known(selector: &str) -> bool {
    if let Some(n) = selector.strip_prefix("dwarf-") {
        // 5 is the newest version GCC 14.2 and 16.2 emit.
        return matches!(n, "2" | "3" | "4" | "5");
    }
    if let Some(method) = selector.strip_prefix("z=") {
        return matches!(method, "none" | "zlib" | "zlib-gnu" | "zlib-gabi" | "zstd");
    }
    if let Some(views) = selector.strip_prefix("variable-location-views=") {
        return views == "incompat5";
    }
    matches!(
        selector,
        "dwarf"
            | "dwarf32"
            | "dwarf64"
            | "strict-dwarf"
            | "split-dwarf"
            | "z"
            | "record-gcc-switches"
            | "column-info"
            | "stabs"
            | "stabs+"
            | "xcoff"
            | "xcoff+"
            | "coff"
            | "coff1"
            | "coff2"
            | "coff3"
            | "vms"
            | "pubnames"
            | "gnu-pubnames"
            | "statement-frontiers"
            | "variable-location-views"
            | "internal-reset-location-views"
            | "describe-dies"
            | "codeview"
            | "btf"
            | "ctf"
            | "as-loc-support"
            | "gas-locview-support"
            | "inline-points"
            | "toggle"
    )
}

/// Is `modifier` (the text after `-gno-`) a debug feature GNU lets a build
/// switch off?
///
/// Such a request is satisfied by construction here: LCCC implements none of
/// these features, so "off" is the actual state.  An unknown modifier is a GNU
/// error (`-gno-bogus` exits 1 on GCC 14.2 and 16.2, measured), which is why
/// this is a closed list rather than the blanket accept it used to be.
fn gnu_debug_feature_off_known(modifier: &str) -> bool {
    matches!(
        modifier,
        "record-gcc-switches"
            | "column-info"
            | "pubnames"
            | "statement-frontiers"
            | "variable-location-views"
            | "internal-reset-location-views"
            | "describe-dies"
            | "strict-dwarf"
            | "inline-points"
    )
}

/// The deliberately *tolerated* class of unrecognized spelling.
///
/// Two measured GNU behaviours are worth keeping, because rejection would
/// break real builds without protecting anything:
///
/// * `-Wno-<name>`: all four reference compilers accept an unknown one
///   silently (build systems disable warnings that may not exist).
/// * `--param <name>=<v>` / `-g<selector>`: the request tunes an internal
///   heuristic (or selects a debug-info flavour) whose meaning lccc does not
///   implement; the program is still correct in either case, and gcc's own
///   parameter/debug-selector tables are not a contract lccc can check
///   against.  GCC *does* validate them (`--param bogus=1`, `-gbogus`,
///   `-gdwarf-9` all exit 1 on GCC 14.2, measured), so this is a deliberate,
///   diagnosed divergence in the permissive direction -- the diagnostic is
///   what keeps it from being a silent one.
///
/// * unknown `-W<name>` / `-Werror=<name>` and the benign presentation
///   presentation namespaces, and anything else the driver has no arm for:
///   LCCC has no table to validate against, and the measurement above shows
///   the spellings real builds pass are overwhelmingly optimisation,
///   reporting and debug-info preferences -- see the module header.
///
/// These are *tolerated*, not *supported*: the diagnostic names them, and
/// `LCCC_STRICT_OPTIONS=1` makes them fatal so a capability probe can
/// establish the difference from exit status alone.
pub fn unknown_option_tolerated(arg: &str, strict: bool) -> Result<(), String> {
    if strict {
        return Err(format!(
            "unrecognized command-line option '{}'; LCCC_STRICT_OPTIONS is set",
            arg
        ));
    }
    eprintln!(
        "warning: unrecognized command-line option '{}' is tolerated but not \
         implemented by LCCC (nothing will act on it); set LCCC_STRICT_OPTIONS=1 \
         to make this an error",
        arg
    );
    Ok(())
}

/// `-D_FORTIFY_SOURCE=2`-style hardening request: accepted, diagnosed,
/// never honoured (yet).
///
/// The macro is undefined for every translation unit by the driver, because
/// glibc's `_FORTIFY_SOURCE` headers expand to `extern __inline __attribute__
/// ((__always_inline__))` wrappers built on `__builtin_va_arg_pack()` and
/// `__builtin_object_size()`.  Modelling that surface is a separate work
/// item; until it is verified end to end, a build that asks for a fortified
/// libc must be told it is not getting one instead of finding out from a
/// security review.  Tolerated by default for source compatibility, fatal
/// under `LCCC_STRICT_OPTIONS=1`.
pub fn fortify_source_unsupported(strict: bool) -> Result<(), String> {
    let msg = "_FORTIFY_SOURCE is not implemented by LCCC; the macro is undefined for \
               this translation unit, so library calls are NOT fortified";
    if strict {
        return Err(format!("{}; LCCC_STRICT_OPTIONS is set", msg));
    }
    eprintln!(
        "lccc: warning: {} (set LCCC_STRICT_OPTIONS=1 to make this an error)",
        msg
    );
    Ok(())
}

/// Every C dialect spelling LCCC recognises, in `-std=` wire form.
///
/// Used for the "did you mean" suggestion on an unrecognized `-std=` value and
/// by tests that pin the accepted set. Kept in one place so a newly taught
/// dialect cannot be added to [`std_version_macro`] without the suggestion
/// machinery seeing it.
pub const STD_DIALECTS: &[&str] = &[
    "c89",
    "c90",
    "c99",
    "c9x",
    "c11",
    "c1x",
    "c17",
    "c18",
    "c23",
    "c2x",
    "c2y",
    "gnu89",
    "gnu90",
    "gnu99",
    "gnu9x",
    "gnu11",
    "gnu1x",
    "gnu17",
    "gnu18",
    "gnu23",
    "gnu2x",
    "gnu2y",
    "iso9899:1990",
    "iso9899:199409",
    "iso9899:1999",
    "iso9899:199x",
    "iso9899:2011",
    "iso9899:2017",
    "iso9899:2018",
];

/// Levenshtein edit distance between two ASCII strings.
///
/// Two-row dynamic program: `O(min(a,b))` space, `O(a*b)` time. The tables are
/// a few dozen bytes (see [`STD_DIALECTS`]) so the quadratic term is
/// irrelevant next to process startup, and `-std=` is parsed once per
/// invocation.
fn edit_distance(a: &str, b: &str) -> usize {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.is_empty() {
        return b.len();
    }
    if b.is_empty() {
        return a.len();
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for (i, &ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, &cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            cur[j + 1] = (prev[j + 1] + 1).min(cur[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// Closest recognised `-std=` dialect to `std_value`, for a GCC-style
/// "did you mean" hint. `None` when nothing is close enough to be useful.
///
/// GCC suggests by edit distance over its dialect table; the same oracle is
/// used here so `-std=c17x`, `-std=gnu1z` and `-std=c++11` produce a pointer
/// instead of a bare rejection. Candidates are restricted to dialects sharing
/// the first character so `-std=z89` is not "corrected" to `-std=c89`, and a
/// hit further than [`STD_SUGGEST_MAX_DISTANCE`] away is suppressed rather
/// than guessed at -- a confidently wrong suggestion is worse than none.
pub fn closest_std_dialect(std_value: &str) -> Option<&'static str> {
    let first = std_value.as_bytes().first().copied()?;
    STD_DIALECTS
        .iter()
        .copied()
        .filter(|d| d.as_bytes().first().copied() == Some(first))
        .map(|d| {
            (
                edit_distance(d, std_value),
                std::cmp::Reverse(common_prefix_len(d, std_value)),
                d,
            )
        })
        // Ties break on the LONGEST shared prefix: a typed extra/missing
        // character (`-std=c17x` for `c17`) keeps the prefix intact, while a
        // shorter unrelated alias (`c1x`) does not. `min_by_key` then keeps the
        // first of any remaining tie, i.e. `STD_DIALECTS` order -- stable.
        .filter(|(dist, _, _)| *dist <= STD_SUGGEST_MAX_DISTANCE)
        .min_by_key(|(dist, prefix, _)| (*dist, *prefix))
        .map(|(_, _, d)| d)
}

/// Number of leading bytes `a` and `b` agree on.
fn common_prefix_len(a: &str, b: &str) -> usize {
    a.bytes().zip(b.bytes()).take_while(|(x, y)| x == y).count()
}

/// Largest edit distance still worth suggesting.
///
/// A fixed budget is used rather than a length-relative one: the dialect
/// lexicon is dense enough (`c99`/`c9x`, `c17`/`c18`, `gnu2x`/`gnu2y`) that a
/// ratio-based budget would happily "correct" a genuine dialect into a
/// near-miss neighbour. Four edits covers every realistic transposition or
/// truncated-word typo while leaving unrelated spellings alone.
const STD_SUGGEST_MAX_DISTANCE: usize = 4;

/// `__STDC_VERSION__` value for a `-std=` dialect name, mirroring GCC.
///
/// `Some(None)` means "the dialect defines no `__STDC_VERSION__` at all"
/// (C89/C90 predate the macro); `None` means the name is unrecognized and the
/// default is left untouched.
pub fn std_version_macro(std_value: &str) -> Option<Option<&'static str>> {
    let v = match std_value {
        "c89" | "c90" | "gnu89" | "gnu90" | "iso9899:1990" => None,
        // The 1994 amendment is the first to define the macro.
        "iso9899:199409" => Some("199409L"),
        "c99" | "c9x" | "gnu99" | "gnu9x" | "iso9899:1999" | "iso9899:199x" => Some("199901L"),
        "c11" | "c1x" | "gnu11" | "gnu1x" | "iso9899:2011" => Some("201112L"),
        "c17" | "c18" | "gnu17" | "gnu18" | "iso9899:2017" | "iso9899:2018" => Some("201710L"),
        "c23" | "c2x" | "gnu23" | "gnu2x" => Some("202311L"),
        // C2y is still a draft; GCC 15 reports 202400L for it.
        "c2y" | "gnu2y" => Some("202400L"),
        _ => return None,
    };
    Some(v)
}

#[cfg(test)]
mod cli_tests {
    use super::cmp_version;
    use crate::backend::Target;
    use crate::common::fp_contract::FpContract;
    use crate::driver::pipeline::Driver;

    /// Parse one flag against a fresh Driver, returning the error text if any.
    fn try_flag(flag: &str) -> Result<(), String> {
        let mut d = Driver::new();
        // args[0] is argv[0] (used for target detection from the binary name).
        let args = vec!["ccc".to_string(), flag.to_string(), "x.c".to_string()];
        d.parse_cli_args(&args).map(|_| ())
    }

    #[test]
    fn ipa_pta_and_live_range_shrinkage_are_real_last_wins_options() {
        for flag in [
            "-fipa-pta",
            "-fno-ipa-pta",
            "-flive-range-shrinkage",
            "-fno-live-range-shrinkage",
        ] {
            try_flag(flag).unwrap_or_else(|e| panic!("{flag} must be implemented: {e}"));
        }
        let mut d = Driver::new();
        let args = [
            "ccc",
            "-fipa-pta",
            "-flive-range-shrinkage",
            "-fno-ipa-pta",
            "-fno-live-range-shrinkage",
            "-fipa-pta",
            "-flive-range-shrinkage",
            "x.c",
        ]
        .map(str::to_string);
        d.parse_cli_args(&args)
            .expect("implemented optimization options");
        assert!(d.ipa_pta);
        assert!(d.live_range_shrinkage);
        let off = ["ccc", "-fno-ipa-pta", "-fno-live-range-shrinkage", "x.c"].map(str::to_string);
        d.parse_cli_args(&off).expect("disabling forms");
        assert!(!d.ipa_pta);
        assert!(!d.live_range_shrinkage);
    }

    /// `-w` inhibits every warning and is dominant regardless of position —
    /// measured against GCC 16.2 (2026-10-17): `-w -Wall` and `-Wall -w` are
    /// both completely silent on a file with a `#warning` directive and an
    /// unused variable, while plain `-Wall` warns. A sticky latch, not an
    /// order-dependent toggle.
    #[test]
    fn dash_w_inhibits_all_warnings_in_both_orders() {
        use crate::common::error::WarningKind;
        for args in [
            vec!["ccc", "-w", "x.c"],
            vec!["ccc", "-w", "-Wall", "x.c"],
            vec!["ccc", "-Wall", "-w", "x.c"],
        ] {
            let mut d = Driver::new();
            let owned: Vec<String> = args.iter().map(|s| s.to_string()).collect();
            d.parse_cli_args(&owned)
                .unwrap_or_else(|e| panic!("{:?} must parse: {e}", args));
            for kind in [WarningKind::Cpp, WarningKind::ReturnType] {
                assert!(
                    !d.warning_config.is_enabled(kind),
                    "{:?}: {:?} must be inhibited by -w",
                    args,
                    kind
                );
            }
        }
        // Control: without -w the same warnings stay enabled.
        let mut d = Driver::new();
        let owned: Vec<String> = ["ccc", "-Wall", "x.c"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        d.parse_cli_args(&owned).expect("-Wall must parse");
        assert!(
            d.warning_config
                .is_enabled(crate::common::error::WarningKind::Cpp)
        );
    }

    /// `-Wno-<kind>` disables and `-Wno-error=<kind>` demotes a known
    /// warning (GCC 16.2: `-Wall -Wno-return-type -Wno-cpp` prints nothing;
    /// `-Werror -Wno-error=cpp` keeps `#warning` a warning). Both used to be
    /// dropped before reaching the configuration.
    #[test]
    fn wno_flags_reach_the_warning_configuration() {
        use crate::common::error::WarningKind;
        let d = parse_flags(&["-Wall", "-Wno-return-type", "-Wno-cpp", "-Wno-attributes"])
            .expect("must parse");
        for kind in [
            WarningKind::ReturnType,
            WarningKind::Cpp,
            WarningKind::Attributes,
        ] {
            assert!(
                !d.warning_config.is_enabled(kind),
                "{kind:?} must be disabled"
            );
        }
        assert!(
            d.warning_config
                .is_enabled(WarningKind::IncompatiblePointerTypes)
        );
        // Error state, GCC 16.2 measured: the per-kind request overrides the
        // global -Werror in either order; between the two per-kind requests
        // the last one wins; -Werror=X re-enables a disabled X.
        for (flags, cpp_is_error) in [
            (&["-Werror", "-Wno-error=cpp"][..], false),
            (&["-Wno-error=cpp", "-Werror"][..], false),
            (&["-Werror=cpp", "-Wno-error=cpp"][..], false),
            (&["-Wno-error=cpp", "-Werror=cpp"][..], true),
            (&["-Werror"][..], true),
        ] {
            let d = parse_flags(flags).expect("must parse");
            assert_eq!(
                d.warning_config.is_error(WarningKind::Cpp),
                cpp_is_error,
                "{flags:?}"
            );
            assert!(d.warning_config.is_enabled(WarningKind::Cpp), "{flags:?}");
            // The demotion is per kind: a global -Werror still covers others.
            if flags.contains(&"-Werror") {
                assert!(
                    d.warning_config.is_error(WarningKind::ReturnType),
                    "{flags:?}"
                );
            }
        }
        let d = parse_flags(&["-Wno-cpp", "-Werror=cpp"]).expect("must parse");
        assert!(d.warning_config.is_enabled(WarningKind::Cpp));
        assert!(d.warning_config.is_error(WarningKind::Cpp));
        // Unknown `-Wno-` names stay silently accepted.
        parse_flags(&["-Wno-bogus-name"]).expect("unknown -Wno- is tolerated");
    }

    fn parse_flags(flags: &[&str]) -> Result<Driver, String> {
        let mut d = Driver::new();
        let mut args = vec!["ccc".to_string()];
        args.extend(flags.iter().map(|f| f.to_string()));
        args.push("x.c".to_string());
        d.parse_cli_args(&args).map(|_| d)
    }

    #[test]
    fn tls_models_are_explicit_and_last_value_wins() {
        use crate::ir::reexports::TlsModel;
        for (spelling, expected) in [
            ("local-exec", TlsModel::LocalExec),
            ("initial-exec", TlsModel::InitialExec),
            ("local-dynamic", TlsModel::LocalDynamic),
            ("global-dynamic", TlsModel::GlobalDynamic),
        ] {
            assert_eq!(
                parse_flags(&[&format!("-ftls-model={spelling}")])
                    .map(|driver| driver.tls_model_floor),
                Ok(Some(expected))
            );
        }
        assert_eq!(
            parse_flags(&[
                "-ftls-model=global-dynamic",
                "-ftls-model=local-exec",
                "-ftls-model=initial-exec",
            ])
            .map(|driver| driver.tls_model_floor),
            Ok(Some(TlsModel::InitialExec))
        );
        // GCC 16.2's diagnostics, measured.
        assert_eq!(
            parse_flags(&["-ftls-model=bogus"]).map(|_| ()),
            Err(
                "unknown TLS model 'bogus'; valid arguments to '-ftls-model=' are: \
                 global-dynamic initial-exec local-dynamic local-exec"
                    .to_string()
            )
        );
        assert_eq!(
            parse_flags(&["-ftls-model="]).map(|_| ()),
            Err("missing argument to '-ftls-model='".to_string())
        );
    }

    #[cfg(feature = "gcc_m16")]
    #[test]
    fn gcc_m16_delegate_retains_gcc_tls_models_in_any_order() {
        for flags in [
            ["-m16", "-ftls-model=global-dynamic"],
            ["-ftls-model=local-dynamic", "-m16"],
        ] {
            assert_eq!(parse_flags(&flags).map(|_| ()), Ok(()), "{flags:?}");
        }
    }

    /// Profile paths, as GCC 16.2 places the `.gcda` files (measured): the
    /// later of `-fprofile-dir` and an explicit mode path wins, and a bare
    /// mode switch takes `-fprofile-dir` from either side.
    #[test]
    fn profile_dir_applies_in_either_order() {
        let generate_dir = |flags: &[&str]| parse_flags(flags).unwrap().pgo_generate;
        let dir = |s: &str| Some(s.to_string());
        assert_eq!(
            generate_dir(&["-fprofile-dir=D", "-fprofile-generate"]),
            dir("D")
        );
        assert_eq!(
            generate_dir(&["-fprofile-generate", "-fprofile-dir=D"]),
            dir("D")
        );
        assert_eq!(
            generate_dir(&["-fprofile-generate=P", "-fprofile-dir=D"]),
            dir("D")
        );
        assert_eq!(
            generate_dir(&["-fprofile-dir=D", "-fprofile-generate=P"]),
            dir("P")
        );
        assert_eq!(
            generate_dir(&["-fprofile-dir=D", "-fprofile-arcs"]),
            dir("D")
        );
        assert_eq!(generate_dir(&["-fprofile-generate"]), dir("."));
        let used = parse_flags(&["-fprofile-dir=U", "-fprofile-use"])
            .unwrap()
            .pgo_use;
        assert_eq!(used, dir("U"));
    }

    #[test]
    fn empty_profile_paths_and_unknown_update_methods_are_errors() {
        for prefix in [
            "-fprofile-generate=",
            "-fprofile-use=",
            "-fprofile-dir=",
            "-fauto-profile=",
            "-fprofile-update=",
        ] {
            assert_eq!(
                try_flag(prefix),
                Err(format!("missing argument to '{prefix}'"))
            );
        }
        let err = try_flag("-fprofile-update=unknown").unwrap_err();
        assert!(
            err.starts_with("unknown profile update method 'unknown'"),
            "{err}"
        );
        for (flag, method) in [
            ("-fprofile-update=single", "single"),
            ("-fprofile-update=atomic", "atomic"),
            ("-fprofile-update=prefer-atomic", "atomic"),
        ] {
            let d = parse_flags(&[flag]).unwrap();
            assert_eq!(d.pgo_update.as_deref(), Some(method), "{flag}");
        }
    }

    #[test]
    fn hosted_freestanding_and_builtin_flags_are_explicit_and_ordered() {
        // The blanket builtin state lives in `builtin_policy`; a hosted alias
        // spelling (`strlen`) is the observable.
        let no_builtins = |d: &Driver| !d.builtin_policy.may_assume("strlen");
        let mut d = Driver::new();
        d.parse_cli_args(&["ccc".into(), "-ffreestanding".into(), "x.c".into()])
            .unwrap();
        assert!(d.freestanding);
        assert!(no_builtins(&d));

        let mut d = Driver::new();
        d.parse_cli_args(&[
            "ccc".into(),
            "-ffreestanding".into(),
            "-fbuiltin".into(),
            "x.c".into(),
        ])
        .unwrap();
        assert!(d.freestanding);
        assert!(!no_builtins(&d));

        let mut d = Driver::new();
        d.parse_cli_args(&[
            "ccc".into(),
            "-fhosted".into(),
            "-fno-builtin".into(),
            "x.c".into(),
        ])
        .unwrap();
        assert!(!d.freestanding);
        assert!(no_builtins(&d));

        let mut d = Driver::new();
        d.parse_cli_args(&[
            "ccc".into(),
            "-fno-builtin".into(),
            "-ffreestanding".into(),
            "-fhosted".into(),
            "x.c".into(),
        ])
        .unwrap();
        assert!(!d.freestanding);
        assert!(!no_builtins(&d));
    }

    #[test]
    fn tree_vectorize_flags_are_explicit_and_last_wins() {
        let mut d = Driver::new();
        d.parse_cli_args(&["ccc".into(), "-fno-tree-vectorize".into(), "x.c".into()])
            .unwrap();
        assert!(d.disable_tree_vectorize);

        d.parse_cli_args(&["ccc".into(), "-ftree-vectorize".into(), "x.c".into()])
            .unwrap();
        assert!(!d.disable_tree_vectorize);
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
        // `-mapx`/`-mapxf` are gated codegen (EGPR + NDD), off unless asked.
        // `-mno-apx` is the default and must stay a no-op.
        for f in ["-mapx", "-mapxf", "-mno-apx", "-mno-apxf"] {
            assert!(try_flag(f).is_ok(), "{f} must be accepted");
        }
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
            // The specific diagnostic, not merely an error: the `-m` forms
            // used to fall into the generic `-m` catch-all ("refuses to
            // silently ignore target-affecting -m flags"), which also errors,
            // so an `is_err()` check could not see their arm was dead.
            let err = try_flag(f).expect_err(f);
            assert!(
                err.contains("does not implement stack-protector canaries"),
                "{f}: wrong diagnostic: {err}"
            );
        }
        // The opposite request is exactly what LCCC already does.
        assert!(try_flag("-fno-stack-protector").is_ok());
    }

    /// `-ftls-model=` / `-mtls-dialect=` diagnostics follow GCC 16.2's
    /// measured wording (error + valid-argument note folded into one line),
    /// and the dialect is validated against the FINAL target (`-m32` given
    /// after it still counts). The dialect arm must precede the generic `-m`
    /// refusal; the native-dialect `Ok` cases pin that ordering.
    #[test]
    fn tls_option_diagnostics_match_gcc() {
        let err = |flags: &[&str]| parse_flags(flags).err().unwrap_or_default();
        assert!(parse_flags(&["-ftls-model=initial-exec"]).is_ok());
        assert!(parse_flags(&["-ftls-model=local-exec", "-ftls-model=global-dynamic"]).is_ok());
        assert_eq!(err(&["-ftls-model="]), "missing argument to '-ftls-model='");
        assert_eq!(
            err(&["-ftls-model=bogus"]),
            "unknown TLS model 'bogus'; valid arguments to '-ftls-model=' are: \
             global-dynamic initial-exec local-dynamic local-exec"
        );
        for ok in [&["-mtls-dialect=gnu2"][..], &["-mtls-dialect=gnu2", "-m32"]] {
            assert!(parse_flags(ok).is_ok(), "{ok:?}: {}", err(ok));
        }
        assert_eq!(
            err(&["-mtls-dialect=foo"]),
            "unrecognized argument in option '-mtls-dialect=foo'; valid arguments to \
             '-mtls-dialect=' are: gnu gnu2"
        );
        // Cross targets come from argv[0], whose parse also flips the
        // process-global char signedness under concurrently running tests;
        // the validator itself is target-parametric, so test it directly.
        assert!(super::check_tls_dialect(&crate::backend::Target::Aarch64, "desc").is_ok());
        assert!(super::check_tls_dialect(&crate::backend::Target::Riscv64, "trad").is_ok());
        assert_eq!(
            super::check_tls_dialect(&crate::backend::Target::Aarch64, "gnu2").unwrap_err(),
            "unrecognized argument in option '-mtls-dialect=gnu2'; valid arguments to \
             '-mtls-dialect=' are: trad desc"
        );
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

    /// `-mfma` implies the VEX encoding world (GCC oracle, gcc 14.2
    /// `-Q --help=target`): `-march=x86-64 -mfma` reports avx, sse4.1,
    /// sse4.2 ENABLED with avx2 still disabled; `-mavx` alone leaves fma
    /// disabled (one-directional). FMA3 has no legacy-SSE encoding, so
    /// this is a hardware truth, not a policy choice. Asserted on the
    /// final ISA the pipeline consults (`x86_isa`), not the raw flags.
    #[test]
    fn mfma_implies_vex_chain_gcc_exact() {
        let mut d = Driver::new();
        let args: Vec<String> = ["ccc", "-march=x86-64-v2", "-mfma", "x.c"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(d.parse_cli_args(&args).is_ok());
        let isa = d.x86_isa();
        assert!(isa.fma, "-mfma at v2 must enable the FMA families");
        assert!(
            isa.avx,
            "-mfma must imply the VEX.128 encoding (no legacy FMA)"
        );
        assert!(isa.sse41, "-mfma must pull the -mavx prerequisite chain");
        assert!(
            !isa.ymm,
            "-mfma must NOT enable the 256-bit class (gcc: avx2 disabled)"
        );
    }

    #[test]
    fn sse2_denial_survives_independent_sse1_codegen_grant() {
        for flags in [
            &["-mno-sse2", "-msse"][..],
            &["-mno-sse2"],
            &["-msse", "-mno-sse2"],
            &["-msse", "-mno-sse2", "-msse"],
            &["-march=x86-64-v3", "-mno-sse2", "-msse"],
        ] {
            let mut d = Driver::new();
            let args: Vec<String> = std::iter::once("ccc")
                .chain(flags.iter().copied())
                .chain(std::iter::once("x.c"))
                .map(str::to_string)
                .collect();
            assert!(d.parse_cli_args(&args).is_ok(), "{flags:?}");
            assert!(!d.no_sse && d.no_sse2, "{flags:?}");
            let isa = d.x86_isa();
            assert!(!isa.simd && !isa.sse41 && !isa.avx && !isa.ymm && !isa.fma);
        }
        let mut d = Driver::new();
        let args = ["ccc", "-mno-sse2", "-msse", "-msse2", "x.c"].map(str::to_string);
        assert!(d.parse_cli_args(&args).is_ok());
        assert!(d.x86_isa().simd);
    }

    /// `-mfma` at the plain SSE2 baseline pulls the whole chain too (GCC
    /// emits FMA there; the march ceiling is not a hard ceiling for
    /// explicit -m feature flags).
    #[test]
    fn mfma_at_sse2_baseline_pulls_chain() {
        let mut d = Driver::new();
        let args: Vec<String> = ["ccc", "-march=x86-64", "-mfma", "x.c"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(d.parse_cli_args(&args).is_ok());
        let isa = d.x86_isa();
        assert!(isa.fma && isa.avx && !isa.ymm);
    }

    /// Reverse order: `-mfma -march=x86-64-v2` — the march level ORs in
    /// (GCC: the earlier -mfma implications survive; verified with
    /// `gcc -mfma -march=x86-64-v2 -Q --help=target`: fma/avx enabled).
    #[test]
    fn mfma_before_march_survives_gcc_exact() {
        let mut d = Driver::new();
        let args: Vec<String> = ["ccc", "-mfma", "-march=x86-64-v2", "x.c"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(d.parse_cli_args(&args).is_ok());
        let isa = d.x86_isa();
        assert!(isa.fma && isa.avx && !isa.ymm);
    }

    /// Later denial wins: `-mno-avx` after `-mfma` kills the chain (GCC:
    /// no FMA emitted under `-march=x86-64-v2 -mfma -mno-avx`).
    #[test]
    fn mno_avx_after_mfma_kills_the_chain() {
        let mut d = Driver::new();
        let args: Vec<String> = ["ccc", "-march=x86-64-v2", "-mfma", "-mno-avx", "x.c"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(d.parse_cli_args(&args).is_ok());
        let isa = d.x86_isa();
        assert!(
            !isa.fma,
            "-mno-avx after -mfma must deny the VEX world (and FMA with it)"
        );
        assert!(!isa.avx);
    }

    /// Later denial wins: `-mno-sse4.1` after `-mfma` kills the chain via
    /// the sse41 prerequisite (GCC: no FMA under
    /// `-march=x86-64 -mfma -mno-sse4.1`).
    #[test]
    fn mno_sse41_after_mfma_kills_the_chain() {
        let mut d = Driver::new();
        let args: Vec<String> = ["ccc", "-march=x86-64", "-mfma", "-mno-sse4.1", "x.c"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(d.parse_cli_args(&args).is_ok());
        let isa = d.x86_isa();
        assert!(!isa.fma, "-mno-sse4.1 after -mfma must veto the chain");
    }

    /// `-mno-sse` then `-mfma`: the explicit feature flag re-enables the
    /// vector file (GCC: -mno-sse -mfma -> sse ENABLED), while the reverse
    /// order keeps everything off. Both directions pinned.
    #[test]
    fn mfma_mno_sse_ordering_matrix() {
        let mut d = Driver::new();
        let args: Vec<String> = ["ccc", "-mno-sse", "-mfma", "x.c"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(d.parse_cli_args(&args).is_ok());
        let isa = d.x86_isa();
        assert!(
            isa.fma && isa.avx,
            "-mfma after -mno-sse re-enables (gcc: sse enabled)"
        );

        let mut d2 = Driver::new();
        let args2: Vec<String> = ["ccc", "-mfma", "-mno-sse", "x.c"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(d2.parse_cli_args(&args2).is_ok());
        assert!(
            !d2.x86_isa().simd,
            "-mno-sse after -mfma kills the vector file"
        );
    }

    /// Regression pin: plain `-march=x86-64-v2` (no -mfma) keeps declining
    /// the families — the implication only fires on the explicit flag.
    #[test]
    fn march_v2_alone_still_declines_fma() {
        let mut d = Driver::new();
        let args: Vec<String> = ["ccc", "-march=x86-64-v2", "x.c"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(d.parse_cli_args(&args).is_ok());
        let isa = d.x86_isa();
        assert!(!isa.fma && !isa.avx && !isa.ymm);
    }

    #[test]
    fn rounding_math_is_independent_of_trapping_and_last_option_wins() {
        for (flags, rounding, trapping) in [
            (vec!["-frounding-math"], true, false),
            (vec!["-frounding-math", "-fno-rounding-math"], false, false),
            (vec!["-fno-rounding-math", "-frounding-math"], true, false),
            (vec!["-frounding-math", "-fno-trapping-math"], true, false),
            (vec!["-ftrapping-math", "-fno-rounding-math"], false, true),
            (vec!["-frounding-math", "-ffast-math"], false, false),
            (vec!["-ffast-math", "-frounding-math"], true, false),
        ] {
            let mut driver = Driver::new();
            let args = std::iter::once("lccc")
                .chain(flags)
                .chain(["x.c"])
                .map(str::to_string)
                .collect::<Vec<_>>();
            driver.parse_cli_args(&args).unwrap();
            assert_eq!(driver.fp_rounding_math, rounding, "{args:?}");
            assert_eq!(driver.fp_trapping_math, trapping, "{args:?}");
        }
    }

    #[test]
    fn fp_trapping_option_has_explicit_last_option_precedence() {
        for (flags, expected) in [
            (vec!["-O3"], false),
            (vec!["-ftrapping-math"], true),
            (vec!["-ftrapping-math", "-fno-trapping-math"], false),
            (vec!["-fno-trapping-math", "-ftrapping-math"], true),
            (vec!["-ffast-math", "-ftrapping-math"], true),
            (vec!["-ftrapping-math", "-ffast-math"], false),
            (vec!["-ffast-math", "-fno-fast-math"], false),
        ] {
            let mut driver = Driver::new();
            let args = std::iter::once("lccc")
                .chain(flags)
                .chain(["x.c"])
                .map(str::to_string)
                .collect::<Vec<_>>();
            driver.parse_cli_args(&args).unwrap();
            assert_eq!(driver.fp_trapping_math, expected, "{args:?}");
        }
    }

    #[test]
    fn wrap_trapping_and_rounding_flags_are_independent() {
        for wraps in [false, true] {
            for trapping in [false, true] {
                for rounding in [false, true] {
                    let flags = [
                        if wraps { "-fwrapv" } else { "-fno-wrapv" },
                        if trapping {
                            "-ftrapping-math"
                        } else {
                            "-fno-trapping-math"
                        },
                        if rounding {
                            "-frounding-math"
                        } else {
                            "-fno-rounding-math"
                        },
                    ];
                    for reverse in [false, true] {
                        let mut ordered = flags.to_vec();
                        if reverse {
                            ordered.reverse();
                        }
                        let args: Vec<String> = std::iter::once("lccc")
                            .chain(ordered)
                            .chain(["x.c"])
                            .map(str::to_owned)
                            .collect();
                        let mut driver = Driver::new();
                        driver.parse_cli_args(&args).unwrap();
                        assert_eq!(driver.wrap_signed_overflow, wraps, "{args:?}");
                        assert_eq!(driver.fp_trapping_math, trapping, "{args:?}");
                        assert_eq!(driver.fp_rounding_math, rounding, "{args:?}");
                    }
                }
            }
        }
    }

    /// The GCC 16.2 table, measured on the oracle with
    /// `memset(b, 7, 8)` (call vs inline) and `__STDC_HOSTED__`.
    #[test]
    fn builtin_flags_match_gcc() {
        for (flags, memset_builtin, hosted) in [
            (vec![], true, true),
            (vec!["-fno-builtin"], false, true),
            (vec!["-fno-builtin", "-fbuiltin"], true, true),
            (vec!["-fno-builtin-memset"], false, true),
            (vec!["-fno-builtin-memset", "-fbuiltin"], false, true),
            (vec!["-ffreestanding"], false, false),
            (vec!["-ffreestanding", "-fhosted"], true, true),
            (vec!["-ffreestanding", "-fbuiltin"], true, false),
            (vec!["-fno-builtin", "-fhosted"], true, true),
            (vec!["-fno-hosted"], false, false),
            (vec!["-fno-freestanding"], true, true),
            (vec!["-fno-builtin-printf"], true, true),
        ] {
            let mut driver = Driver::new();
            let args = std::iter::once("lccc")
                .chain(flags)
                .chain(["x.c"])
                .map(str::to_string)
                .collect::<Vec<_>>();
            driver.parse_cli_args(&args).unwrap();
            assert_eq!(
                driver.builtin_policy.may_assume("memset"),
                memset_builtin,
                "{args:?}"
            );
            assert_eq!(!driver.freestanding, hosted, "{args:?}");
        }
    }

    #[test]
    fn signed_wrap_option_is_explicit_and_last_wins() {
        for (flags, expected) in [
            (vec!["-O3"], false),
            (vec!["-fwrapv"], true),
            (vec!["-fwrapv", "-fno-wrapv"], false),
            (vec!["-fno-wrapv", "-fwrapv"], true),
            (vec!["-fwrapv", "-ffast-math"], true),
        ] {
            let mut driver = Driver::new();
            let args = std::iter::once("lccc")
                .chain(flags)
                .chain(["x.c"])
                .map(str::to_string)
                .collect::<Vec<_>>();
            driver.parse_cli_args(&args).unwrap();
            assert_eq!(driver.wrap_signed_overflow, expected, "{args:?}");
        }
    }

    /// `-fmath-errno` is the C default; the umbrella and the explicit flags
    /// are plain last-option-wins, exactly as measured on GCC 16.2 (sqrt of a
    /// negative volatile operand setting `errno`).
    #[test]
    fn math_errno_defaults_on_and_last_option_wins() {
        for (flags, expected) in [
            (&[][..], true),
            (&["-fno-math-errno"][..], false),
            (&["-ffast-math"][..], false),
            (&["-ffast-math", "-fmath-errno"][..], true),
            (&["-fmath-errno", "-ffast-math"][..], false),
            (&["-fno-math-errno", "-fno-fast-math"][..], true),
            (&["-ffast-math", "-fno-fast-math"][..], true),
        ] {
            let mut driver = Driver::new();
            let args = std::iter::once("lccc")
                .chain(flags.iter().copied())
                .chain(["x.c"])
                .map(str::to_string)
                .collect::<Vec<_>>();
            driver.parse_cli_args(&args).unwrap();
            assert_eq!(driver.math_errno, expected, "{flags:?}");
        }
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

    /// GCC parity at the low optimization levels (measured, gcc 14.2):
    /// `-O0`/`-O1` never contract `a*b+c` into an FMA, even with
    /// `-march=x86-64-v3`; fusion starts at `-O2`.  The C-language default
    /// (`-ffp-contract=fast`) must therefore not apply below `-O2`, while an
    /// explicit flag still wins at every level.
    #[test]
    fn fp_contract_default_is_off_below_o2_and_explicit_wins() {
        let mut d = Driver::new();
        d.parse_cli_args(&["lccc".to_string(), "-O0".to_string(), "x.c".to_string()])
            .unwrap();
        assert!(
            d.resolved_fp_contract() == FpContract::Off,
            "-O0 must not contract by default (GCC parity)"
        );
        let mut d = Driver::new();
        d.parse_cli_args(&["lccc".to_string(), "-O1".to_string(), "x.c".to_string()])
            .unwrap();
        assert!(
            d.resolved_fp_contract() == FpContract::Off,
            "-O1 must not contract by default"
        );

        let mut d = Driver::new();
        d.parse_cli_args(&["lccc".to_string(), "-O2".to_string(), "x.c".to_string()])
            .unwrap();
        assert!(
            d.resolved_fp_contract() == FpContract::Fast,
            "-O2 keeps GCC's fast contraction default"
        );

        let mut d = Driver::new();
        d.parse_cli_args(&[
            "lccc".to_string(),
            "-O0".to_string(),
            "-ffp-contract=fast".to_string(),
            "x.c".to_string(),
        ])
        .unwrap();
        assert!(
            d.resolved_fp_contract() == FpContract::Fast,
            "an explicit -ffp-contract=fast must win at -O0"
        );

        let mut d = Driver::new();
        d.parse_cli_args(&[
            "lccc".to_string(),
            "-O3".to_string(),
            "-ffp-contract=off".to_string(),
            "x.c".to_string(),
        ])
        .unwrap();
        assert!(
            d.resolved_fp_contract() == FpContract::Off,
            "explicit off wins at -O3"
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
    fn rounding_policy_is_independent_and_last_wins_redteam() {
        for (flags, rounding, trapping) in [
            (vec!["-frounding-math", "-fno-trapping-math"], true, false),
            (vec!["-ftrapping-math", "-fno-rounding-math"], false, true),
            (vec!["-frounding-math", "-fno-rounding-math"], false, false),
            (vec!["-fno-rounding-math", "-frounding-math"], true, false),
            (vec!["-frounding-math", "-ffast-math"], false, false),
            (vec!["-ffast-math", "-frounding-math"], true, false),
        ] {
            let mut driver = Driver::new();
            let args: Vec<String> = std::iter::once("lccc")
                .chain(flags)
                .chain(std::iter::once("x.c"))
                .map(str::to_owned)
                .collect();
            driver.parse_cli_args(&args).unwrap();
            assert_eq!(driver.fp_rounding_math, rounding, "{args:?}");
            assert_eq!(driver.fp_trapping_math, trapping, "{args:?}");
        }
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
        assert!(
            d2.linker_ordered_items
                .iter()
                .any(|i| i == "-Wl,--export-dynamic")
        );
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
        assert!(
            d.linker_ordered_items
                .iter()
                .any(|i| i == "-Wl,-e,__libc_main")
        );
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

    #[test]
    fn i686_bmi2_builtin_permission_does_not_enable_unsupported_codegen() {
        for (flags, permitted) in [
            (vec!["-mbmi2"], true),
            (vec!["-mbmi2", "-mno-bmi2"], false),
            (vec!["-mno-bmi2", "-mbmi2"], true),
        ] {
            let mut driver = Driver::new();
            let args: Vec<String> = ["ccc", "-m32"]
                .into_iter()
                .chain(flags)
                .chain(["x.c"])
                .map(str::to_string)
                .collect();
            assert!(driver.parse_cli_args(&args).is_ok());
            assert_eq!(driver.resolved_builtin_bmi2(), permitted);
            assert!(!driver.resolved_bmi2());
        }
    }

    /// The absent-`-march` baseline is plain x86-64 with no BMI1/BMI2, so a
    /// default build must NOT emit ANDN/RORX-class code.  BMI1/BMI2 are v3
    /// members and are granted only by an explicit `-march=x86-64-v3` or `-mbmi*`.
    #[test]
    fn bmi_is_opt_in_not_default() {
        let mut d = Driver::new();
        let args: Vec<String> = ["ccc", "x.c"].iter().map(|s| s.to_string()).collect();
        assert!(d.parse_cli_args(&args).is_ok());
        assert!(
            !d.resolved_bmi1(),
            "default x86-64 baseline must not carry BMI1"
        );
        assert!(
            !d.resolved_bmi2(),
            "default x86-64 baseline must not carry BMI2"
        );
        let mut v3 = Driver::new();
        let args3: Vec<String> = ["ccc", "-march=x86-64-v3", "x.c"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(v3.parse_cli_args(&args3).is_ok());
        assert!(
            v3.resolved_bmi1() && v3.resolved_bmi2(),
            "-march=x86-64-v3 grants BMI1/BMI2"
        );
    }

    /// `-mno-bmi` is a STICKY denial against an explicit v3 grant — the
    /// default must not revive the class behind the user's back (GCC keeps
    /// the last-explicit ISA decision).
    #[test]
    fn mno_bmi_sticky_denial_beats_explicit_grant() {
        let mut d = Driver::new();
        let args: Vec<String> = ["ccc", "-march=x86-64-v3", "-mno-bmi", "x.c"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(d.parse_cli_args(&args).is_ok());
        assert!(!d.resolved_bmi1(), "-mno-bmi must deny the v3 grant");
        assert!(d.resolved_bmi2(), "-mno-bmi leaves the BMI2 class alone");

        let mut d2 = Driver::new();
        let args2: Vec<String> = ["ccc", "-march=x86-64-v3", "-mno-bmi2", "x.c"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(d2.parse_cli_args(&args2).is_ok());
        assert!(!d2.resolved_bmi2());
        assert!(d2.resolved_bmi1());
    }

    /// GCC last-explicit ISA semantics: a later `-mbmi` lifts an earlier
    /// `-mno-bmi` sticky denial (mirrors `-msse` after `-mno-sse`).
    #[test]
    fn mbmi_after_mno_bmi_reenables() {
        let mut d = Driver::new();
        let args: Vec<String> = ["ccc", "-mno-bmi", "-mbmi", "x.c"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(d.parse_cli_args(&args).is_ok());
        assert!(d.resolved_bmi1());
    }

    /// CMPXCHG16B gates the inline 16-byte compare-exchange: the default
    /// baseline must not carry it (a default build's 16-byte CAS calls
    /// libatomic), `-mcx16` and the v2+ profiles grant it, and a sticky
    /// `-mno-cx16` denies it in either order.  It is integer ISA, so
    /// `-mno-sse` leaves it alone.
    #[test]
    fn cx16_is_opt_in_not_default() {
        let parse = |flags: &[&str]| {
            let mut d = Driver::new();
            let mut args: Vec<String> = vec![String::from("ccc")];
            args.extend(flags.iter().map(|s| s.to_string()));
            args.push(String::from("x.c"));
            assert!(d.parse_cli_args(&args).is_ok(), "{flags:?}");
            d
        };
        assert!(
            !parse(&[]).resolved_cx16(),
            "default baseline must not carry CMPXCHG16B"
        );
        assert!(
            !parse(&[]).x86_isa().cx16,
            "default ISA permission must not carry CMPXCHG16B"
        );
        assert!(
            parse(&["-mcx16"]).resolved_cx16(),
            "-mcx16 grants CMPXCHG16B"
        );
        assert!(
            parse(&["-mcx16"]).x86_isa().cx16,
            "-mcx16 reaches the emitter"
        );
        assert!(
            parse(&["-march=x86-64-v2"]).resolved_cx16(),
            "x86-64-v2 carries CMPXCHG16B"
        );
        assert!(
            parse(&["-march=x86-64-v3"]).resolved_cx16(),
            "x86-64-v3 carries CMPXCHG16B"
        );
        assert!(
            !parse(&["-march=x86-64-v3", "-mno-cx16"]).resolved_cx16(),
            "-mno-cx16 must deny the v3 grant"
        );
        assert!(
            !parse(&["-mno-cx16", "-march=x86-64-v2"]).resolved_cx16(),
            "-mno-cx16 is sticky against a later v2 profile"
        );
        assert!(
            parse(&["-mno-sse", "-mcx16"]).x86_isa().cx16,
            "CMPXCHG16B is integer ISA and survives -mno-sse"
        );
    }

    /// CMPXCHG16B is an x86-64 permission only: the 128-bit compare-exchange
    /// lowering lives in the x86-64 backend, so no other target may report it
    /// even with the flag set.  Reporting it elsewhere would promise an inline
    /// `lock cmpxchg16b` that no emitter on that target can produce.
    #[test]
    fn cx16_is_x86_64_only() {
        let mut d = Driver::new();
        d.enable_cx16 = true;
        for target in [
            Target::X86_64,
            Target::I686,
            Target::Aarch64,
            Target::Riscv64,
        ] {
            let want = target == Target::X86_64;
            d.target = target;
            let resolved = d.resolved_cx16();
            assert!(
                resolved == want,
                "{target:?}: resolved_cx16() = {resolved}, expected {want}"
            );
            let perm = d.x86_isa().cx16;
            assert!(
                perm == want,
                "{target:?}: x86_isa().cx16 = {perm}, expected {want}"
            );
        }
        // The sticky denial still outranks the grant on the one target that
        // has the permission.
        d.target = Target::X86_64;
        d.cx16_explicitly_disabled = true;
        assert!(!d.resolved_cx16());
        assert!(!d.x86_isa().cx16);
    }

    /// ABM (LZCNT/TZCNT) is an x86-64-v3 member: only an explicit grant
    /// projects it, and then `tzcnt`/`lzcnt` supersede `bsf`/`bsr` (defined on
    /// zero input).  The default build keeps `bsf`/`bsr`.
    #[test]
    fn lzcnt_is_opt_in_not_default() {
        let mut d = Driver::new();
        let args: Vec<String> = ["ccc", "x.c"].iter().map(|s| s.to_string()).collect();
        assert!(d.parse_cli_args(&args).is_ok());
        assert!(
            !d.resolved_lzcnt(),
            "default x86-64 baseline must not carry ABM"
        );
        let mut v3 = Driver::new();
        let args3: Vec<String> = ["ccc", "-march=x86-64-v3", "x.c"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(v3.parse_cli_args(&args3).is_ok());
        assert!(v3.resolved_lzcnt(), "-march=x86-64-v3 must project ABM");
    }

    /// Sticky `-mno-lzcnt` denial beats an explicit v3 grant; the BMI
    /// classes are unaffected (independent ISA groups).
    #[test]
    fn mno_lzcnt_sticky_denial_beats_explicit_grant() {
        let mut d = Driver::new();
        let args: Vec<String> = ["ccc", "-march=x86-64-v3", "-mno-lzcnt", "x.c"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(d.parse_cli_args(&args).is_ok());
        assert!(!d.resolved_lzcnt(), "-mno-lzcnt must deny the v3 grant");
        assert!(d.resolved_bmi1());
        assert!(d.resolved_bmi2());
    }

    /// GCC last-explicit ISA semantics: a later `-mlzcnt` lifts an earlier
    /// `-mno-lzcnt` sticky denial.
    #[test]
    fn mlzcnt_after_mno_lzcnt_reenables() {
        let mut d = Driver::new();
        let args: Vec<String> = ["ccc", "-mno-lzcnt", "-mlzcnt", "x.c"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(d.parse_cli_args(&args).is_ok());
        assert!(d.resolved_lzcnt());
    }

    /// POPCNT joined the baseline at x86-64-v2, so only an explicit v2+
    /// profile projects it; sticky `-mno-popcnt` wins; a later `-mpopcnt` lifts
    /// the denial; an explicit v1 ceiling removes it.
    #[test]
    fn popcnt_opt_in_sticky_denial_and_ceiling() {
        let mut d = Driver::new();
        let args: Vec<String> = ["ccc", "x.c"].iter().map(|s| s.to_string()).collect();
        assert!(d.parse_cli_args(&args).is_ok());
        assert!(
            !d.resolved_popcnt(),
            "default x86-64 baseline must not carry POPCNT"
        );
        let mut v3 = Driver::new();
        let args_v3: Vec<String> = ["ccc", "-march=x86-64-v3", "x.c"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(v3.parse_cli_args(&args_v3).is_ok());
        assert!(v3.resolved_popcnt(), "-march=x86-64-v3 must project POPCNT");

        let mut d2 = Driver::new();
        let args2: Vec<String> = ["ccc", "-mno-popcnt", "x.c"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(d2.parse_cli_args(&args2).is_ok());
        assert!(!d2.resolved_popcnt(), "-mno-popcnt must deny the v3 grant");

        let mut d3 = Driver::new();
        let args3: Vec<String> = ["ccc", "-mno-popcnt", "-mpopcnt", "x.c"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(d3.parse_cli_args(&args3).is_ok());
        assert!(d3.resolved_popcnt(), "-mpopcnt lifts the sticky denial");

        let mut d4 = Driver::new();
        let args4: Vec<String> = ["ccc", "-march=x86-64", "x.c"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(d4.parse_cli_args(&args4).is_ok());
        assert!(!d4.resolved_popcnt(), "v1 ceiling must not carry POPCNT");
    }

    /// An explicit `-march=x86-64` (v1) ceiling removes ABM from the
    /// default projection — the baseline only applies without `-march`.
    #[test]
    fn explicit_v1_march_removes_lzcnt_baseline() {
        let mut d = Driver::new();
        let args: Vec<String> = ["ccc", "-march=x86-64", "x.c"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(d.parse_cli_args(&args).is_ok());
        assert!(!d.resolved_lzcnt(), "v1 ceiling must not carry ABM");

        let mut d2 = Driver::new();
        let args2: Vec<String> = ["ccc", "-march=x86-64-v3", "x.c"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(d2.parse_cli_args(&args2).is_ok());
        assert!(d2.resolved_lzcnt(), "v3 profile carries ABM");
    }

    /// An explicit `-march=` is the ceiling (GCC-exact): v1 has no BMI, so
    /// the default-baseline BMI dies — unless an explicit `-mbmi` re-adds
    /// the feature on top of the ceiling.
    #[test]
    fn explicit_march_is_the_bmi_ceiling() {
        let mut d = Driver::new();
        let args: Vec<String> = ["ccc", "-march=x86-64", "x.c"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(d.parse_cli_args(&args).is_ok());
        assert!(!d.resolved_bmi1(), "v1 ceiling has no BMI1");
        assert!(!d.resolved_bmi2(), "v1 ceiling has no BMI2");

        let mut d2 = Driver::new();
        let args2: Vec<String> = ["ccc", "-march=x86-64", "-mbmi", "x.c"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(d2.parse_cli_args(&args2).is_ok());
        assert!(d2.resolved_bmi1(), "explicit -mbmi adds the feature");

        let mut d3 = Driver::new();
        let args3: Vec<String> = ["ccc", "-march=x86-64-v3", "x.c"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(d3.parse_cli_args(&args3).is_ok());
        assert!(d3.resolved_bmi1());
        assert!(d3.resolved_bmi2());
    }

    /// A second `-march=` REPLACES the first arch's set (GCC-exact, measured
    /// on gcc 14.2 `-dM -E`: `-march=x86-64-v3 -march=x86-64` leaves
    /// `__BMI__`, `__MOVBE__` and `__AVX__` all absent).  Before the
    /// replace-and-replay contract the profile functions only ever ADDED,
    /// so the first `-march=`'s enables survived the second as a union.
    #[test]
    fn second_march_replaces_the_first_arch_set() {
        let mut d = Driver::new();
        let args: Vec<String> = ["ccc", "-march=x86-64-v3", "-march=x86-64", "x.c"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(d.parse_cli_args(&args).is_ok());
        assert!(!d.resolved_bmi1(), "the v1 ceiling must replace, not union");
        assert!(!d.resolved_movbe());
        assert!(!d.resolved_popcnt());
        assert!(!d.enable_avx, "the SIMD half replaces too");

        // The replacement composes with explicit flags recorded between
        // the two -march=: the replay lands on the FINAL arch set.
        let mut d2 = Driver::new();
        let args2: Vec<String> = ["ccc", "-march=x86-64-v3", "-mavx", "-march=x86-64", "x.c"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(d2.parse_cli_args(&args2).is_ok());
        assert!(d2.enable_avx, "explicit -mavx survives the later -march=");
        assert!(!d2.resolved_bmi1(), "the arch-implied BMI still dies");
    }

    /// GCC explicit-ISA semantics, order-independent (measured on gcc 14.2):
    /// `-mno-bmi -march=x86-64-v3` leaves `__BMI__` ABSENT (the explicit
    /// denial overrides the final arch set in either flag order).  This
    /// refutes the "GCC is strictly last-wins and re-enables BMI1" reading:
    /// the model is explicit-mask-over-arch, not last-wins.
    #[test]
    fn mno_bmi_stays_denied_under_a_later_march_in_either_order() {
        for flags in [
            ["ccc", "-mno-bmi", "-march=x86-64-v3", "x.c"],
            ["ccc", "-march=x86-64-v3", "-mno-bmi", "x.c"],
        ] {
            let mut d = Driver::new();
            let args: Vec<String> = flags.iter().map(|s| s.to_string()).collect();
            assert!(d.parse_cli_args(&args).is_ok());
            assert!(!d.resolved_bmi1(), "{flags:?}: BMI1 must stay denied");
            assert!(d.resolved_bmi2(), "{flags:?}: BMI2 is untouched");
            assert!(d.resolved_movbe(), "{flags:?}: MOVBE is untouched");
        }
    }

    /// GCC explicit-ISA semantics (measured on gcc 14.2): `-mbmi
    /// -march=x86-64` DEFINES `__BMI__` — an explicit enable survives a
    /// lower arch, exactly as an explicit denial survives a higher one.
    #[test]
    fn mbmi_survives_a_lower_march() {
        let mut d = Driver::new();
        let args: Vec<String> = ["ccc", "-mbmi", "-march=x86-64", "x.c"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(d.parse_cli_args(&args).is_ok());
        assert!(d.resolved_bmi1(), "explicit -mbmi survives -march=x86-64");
        assert!(!d.resolved_bmi2(), "BMI2 was never enabled");
    }

    /// SSE4.2 ⇒ POPCNT — the Nehalem bundle (measured on gcc 14.2:
    /// `-msse4.2 -march=x86-64`, `-mavx -march=x86-64`,
    /// `-mavx2 -march=x86-64` and `-mfma -march=x86-64` all define
    /// `__POPCNT__`; `-msse4.1` does NOT).
    #[test]
    fn sse42_family_implies_popcnt() {
        for flag in ["-msse4.2", "-mavx", "-mavx2", "-mfma"] {
            let mut d = Driver::new();
            let args: Vec<String> = ["ccc", flag, "-march=x86-64", "x.c"]
                .iter()
                .map(|s| s.to_string())
                .collect();
            assert!(d.parse_cli_args(&args).is_ok());
            assert!(
                d.resolved_popcnt(),
                "{flag}: the SSE4.2 family must carry POPCNT (GCC Nehalem bundle)"
            );
        }
        // SSE4.1 does NOT imply POPCNT.
        let mut d = Driver::new();
        let args: Vec<String> = ["ccc", "-msse4.1", "-march=x86-64", "x.c"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(d.parse_cli_args(&args).is_ok());
        assert!(!d.resolved_popcnt(), "SSE4.1 alone must not carry POPCNT");
    }

    /// BMI is an INTEGER class: `-mno-sse` denies the xmm world but leaves
    /// ANDN/RORX legal — an explicit `-march=x86-64-v3` baseline still applies.
    #[test]
    fn bmi_survives_mno_sse() {
        let mut d = Driver::new();
        let args: Vec<String> = ["ccc", "-march=x86-64-v3", "-mno-sse", "x.c"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(d.parse_cli_args(&args).is_ok());
        assert!(d.resolved_bmi1(), "integer BMI survives the xmm denial");
        assert!(d.resolved_bmi2());
    }

    // ---- unknown-option policy (module header) --------------------------
    //
    // The contract is measured, not assumed: gcc 14.2 (in this tree) and
    // gcc 16.2 / clang 23.1 / icc 2021.10 / icx (pinned oracles) all fail on
    // `-Wbogus`, `-fno-bogus`, `-gbogus` and `--param bogus=1`, while all
    // four accept `-Wno-bogus` silently.  LCCC matches the compatibility
    // target (GCC): unknown is fatal; the `-Wno-` carve-out is tolerated by
    // default and fatal under LCCC_STRICT_OPTIONS, so a capability probe can
    // still separate "implemented" from "tolerated".

    #[test]
    fn unknown_option_is_fatal_by_default() {
        for arg in ["-fno-strict-aliasing", "--totally-bogus", "-Wbogus"] {
            let err = super::unknown_option(arg, false)
                .expect_err("an unimplemented option must not let a probe read success");
            assert!(err.contains(arg), "{err}");
            assert!(
                err.contains("no implementation"),
                "the diagnostic must say the option is unimplemented: {err}"
            );
        }
    }

    #[test]
    fn unknown_option_is_fatal_under_strict_options() {
        let err = super::unknown_option("-fno-strict-aliasing", true)
            .expect_err("strict mode must reject an unimplemented option");
        assert!(err.contains("-fno-strict-aliasing"), "{err}");
        assert!(err.contains("LCCC_STRICT_OPTIONS"), "{err}");
    }

    #[test]
    fn tolerated_class_warns_by_default_and_fails_under_strict() {
        for arg in ["-Wno-bogus", "--param=ssp-buffer-size=4", "-gdwarf-9"] {
            assert!(
                super::unknown_option_tolerated(arg, false).is_ok(),
                "{arg}: the GNU-tolerated class must not break a build"
            );
            let err = super::unknown_option_tolerated(arg, true)
                .expect_err("strict mode must escalate the tolerated class");
            assert!(err.contains(arg), "{err}");
        }
    }

    /// The `-W` arm must reach the policy: an unknown warning name is
    /// diagnosed, and fatal under strict options (the pre-fix code discarded
    /// `WarningConfig::process_flag`'s `false`, so the name vanished — and the
    /// capability probe read the silence as support).
    #[test]
    fn unknown_warning_option_is_diagnosed_and_strict_fatal() {
        // Default policy: a build that passes a name LCCC has no table for
        // (409 GNU names against the six implemented here) must still build.
        for arg in [
            "-Wbogus",
            "-Werror=bogus",
            "-Wno-error=bogus",
            "-Wsign-compare",
        ] {
            assert!(try_flag(arg).is_ok(), "{arg}");
            let err = super::unknown_option_tolerated(arg, true)
                .expect_err("strict mode must not report an unknown name as implemented");
            assert!(err.contains(arg), "{err}");
        }
        // `-Wno-<name>` is honoured by construction (nothing to switch off), so
        // it needs no diagnostic at all.
        for arg in [
            "-Wno-bogus",
            "-Wno-error",
            "-Wno-error=unused-variable",
            "-Wno-unused-variable",
        ] {
            assert!(try_flag(arg).is_ok(), "{arg}");
        }
        // A *known* warning option is untouched by the change.
        for arg in [
            "-Wall",
            "-Wextra",
            "-Werror",
            "-Wreturn-type",
            "-Wno-return-type",
        ] {
            assert!(try_flag(arg).is_ok(), "{arg}");
        }
    }

    /// The open `-f`/`-m`/`--` namespace: tolerated with a diagnostic, fatal
    /// under strict options.
    ///
    /// The list is not hypothetical. A binary audit of the 93 spellings in
    /// `common_build_system_flags_are_recognised` against a prebuilt driver
    /// found 16 with no arm — the ones below — and every one of them is passed
    /// *unconditionally* by the kernel, glibc, zlib-ng, gzip or expat build
    /// systems. Failing the build on them would break real builds while
    /// protecting nothing: they are optimisation, reporting and debug-info
    /// preferences. The requests whose being ignored would change the
    /// program's meaning are refused by their own arms instead, which is what
    /// the next test pins.
    #[test]
    fn unimplemented_optimisation_flags_are_diagnosed_not_fatal() {
        for arg in [
            "-fno-strict-aliasing",
            "-fstrict-aliasing",
            "-fwrapv",
            "-fno-strict-overflow",
            "-fno-plt",
            "-fno-semantic-interposition",
            "-fmerge-all-constants",
            "-fno-ident",
            "-fshort-enums", // contract tier, checked below
            "-fno-var-tracking",
            "-fvar-tracking-assignments",
            "-funroll-loops",
            "-ftree-vectorize",
            "-fno-tree-vectorize",
            "-fno-stack-clash-protection",
            "-fuse-ld=bfd",
            "-Og",
            "-fdiagnostics-color=always",
            "-fmax-errors=5",
        ] {
            if arg == "-fshort-enums" {
                continue; // its own contract arm, asserted in the next test
            }
            assert!(try_flag(arg).is_ok(), "{arg} must not fail a build");
            assert!(
                super::unknown_option_tolerated(arg, true).is_err(),
                "{arg}: strict mode must not report it as implemented"
            );
        }
        // A spelling no compiler knows is diagnosed the same way; strict mode is
        // what turns it into the refusal a capability probe needs.
        assert!(try_flag("--totally-bogus").is_ok());
        assert!(super::unknown_option_tolerated("--totally-bogus", true).is_err());
    }

    /// The requests that cannot be satisfied by ignoring them are refused, not
    /// diagnosed: stack protector (already), the data model, and the dialect.
    #[test]
    fn meaning_changing_requests_are_refused_not_ignored() {
        for arg in [
            "-fstack-protector",
            "-fstack-protector-strong",
            "-ftrapv",
            "-fsanitize=address",
            "-fshort-enums",
            "-fpack-struct",
            "-fpack-struct=1",
            "-std=obscure-dialect",
        ] {
            assert!(try_flag(arg).is_err(), "{arg} must be refused");
        }
        // ...while the *suppression* spellings of the same families are fine:
        // LCCC already does not do what they ask to switch off.
        for arg in [
            "-fno-stack-protector",
            "-fno-builtin-memcpy",
            "-fsigned-char",
            // Implemented, not merely tolerated (divergence from the closed
            // PR #753, which predated both implementations):
            // `char_is_unsigned()` is honoured by the parser, the
            // preprocessor's predefined macros and lowering;
            // `set_short_wchar` plumbs the 16-bit `wchar_t` request.
            "-funsigned-char",
            "-fshort-wchar",
        ] {
            assert!(try_flag(arg).is_ok(), "{arg}");
        }
    }

    /// `-ansi` is `-std=c90`, not an ignorable spelling: a dialect request
    /// changes what the source is allowed to mean.
    #[test]
    fn ansi_selects_the_c90_dialect() {
        let mut d = Driver::new();
        let args = vec!["ccc".to_string(), "-ansi".to_string(), "x.c".to_string()];
        d.parse_cli_args(&args).expect("-ansi");
        assert!(!d.gnu_extensions, "-ansi is strict ISO, not a GNU dialect");
        assert_eq!(d.stdc_version, super::std_version_macro("c90"));
        // ...and the rest of the command line still parses (the arm must not
        // return early).
        assert_eq!(d.input_files, vec!["x.c".to_string()]);
    }

    /// No flag a real build system passes unconditionally may *fail* the
    /// driver.
    ///
    /// A regression pin for the whole tier policy, and the reason it exists is
    /// measured: an audit of 93 spellings taken from the Linux kernel, glibc,
    /// zlib-ng, gzip, expat and the distro CFLAGS conventions, run against a
    /// prebuilt driver, found 16 with no arm (`-fno-strict-aliasing`,
    /// `-fwrapv`, `-fno-strict-overflow`, `-fno-plt`, `-flto`,
    /// `-funroll-loops`, `-Og`, `-ansi`, `-fno-ident`, `-fuse-ld=*`,
    /// `-fvar-tracking-assignments`, …) plus two, `--sysroot=` and
    /// `-nostartfiles`, whose implementation lives in another layer.  Every one
    /// of them is now implemented, diagnosed or refused only when ignoring it
    /// would change the program; none may abort a build.  A list, not a
    /// principle, because the failure mode is "a real-world spelling nobody
    /// thought of": extend it when a build reports one.
    #[test]
    fn common_build_system_flags_are_recognised() {
        const FLAGS: &[&str] = &[
            // language/dialect
            "-std=gnu11",
            "-std=c17",
            "-ansi",
            // optimisation and codegen
            "-O2",
            "-O3",
            "-Os",
            "-Oz",
            "-Og",
            "-fPIC",
            "-fpic",
            "-fPIE",
            "-fpie",
            "-fno-pic",
            "-fno-PIE",
            "-fno-strict-aliasing",
            "-fstrict-aliasing",
            "-fomit-frame-pointer",
            "-fno-omit-frame-pointer",
            "-ffreestanding",
            "-fhosted",
            "-fno-builtin",
            "-fno-builtin-memcpy",
            "-fwrapv",
            "-fno-strict-overflow",
            "-fno-common",
            "-fsigned-char",
            "-fno-asynchronous-unwind-tables",
            "-fno-unwind-tables",
            "-fasynchronous-unwind-tables",
            "-fno-ident",
            "-fmerge-all-constants",
            "-fno-math-errno",
            "-ffp-contract=off",
            "-fgnu89-inline",
            "-fno-plt",
            "-fno-semantic-interposition",
            "-fno-var-tracking",
            "-fvar-tracking-assignments",
            "-falign-loops=32",
            "-falign-functions=16",
            "-funroll-loops",
            "-ftree-vectorize",
            "-fno-tree-vectorize",
            "-fuse-ld=bfd",
            "-fuse-ld=lld",
            "-march=native",
            "-mtune=generic",
            "-msse4.2",
            "-mavx2",
            "-mno-red-zone",
            "-m64",
            "-m32",
            "-fno-stack-clash-protection",
            // linking / driver
            "-pipe",
            "-pthread",
            "-nostdlib",
            "-nostdinc",
            "-nostartfiles",
            "-nodefaultlibs",
            "--sysroot=/opt/sysroot",
            "-isysroot=/opt/sysroot",
            "-shared",
            "-static",
            "-rdynamic",
            "-no-pie",
            "-L/usr/local/lib",
            "-lm",
            "-lz",
            "-Wl,-z,now",
            "-Wl,--as-needed",
            // diagnostics
            "-Wall",
            "-Wextra",
            "-Werror",
            "-Wno-unused-parameter",
            "-Wno-sign-compare",
            "-fdiagnostics-color=always",
            "-fmax-errors=5",
        ];
        for arg in FLAGS {
            assert!(try_flag(arg).is_ok(), "{arg} must not fail a build");
        }
        // LTO is a link-model request, not an inert preference: ignoring it
        // would read as support to a probe and silently skip the requested
        // optimisation, so it refuses and the build system falls back.
        for arg in ["-flto", "-flto=auto"] {
            assert!(try_flag(arg).is_err(), "{arg} must be refused, not ignored");
        }
    }

    /// Spellings whose implementation lives in another layer must be
    /// *recognised* here.
    ///
    /// `--sysroot=`/`-isysroot` are resolved by the link layer and the
    /// include search; `-nostartfiles`/`-nodefaultlibs` are acted on by the
    /// link layer once forwarded (see
    /// `nostartfiles_and_nodefaultlibs_reach_the_linker`).  An arm-less
    /// spelling is a broken build (a cross build for `--sysroot=`, a
    /// freestanding link for `-nostartfiles`).
    #[test]
    fn options_implemented_in_other_layers_are_recognised() {
        for arg in [
            "--sysroot=/opt/sysroot",
            "-isysroot=/opt/sysroot",
            "-nostartfiles",
            "-nodefaultlibs",
            "-nostdlib",
            "-nostdinc",
        ] {
            assert!(try_flag(arg).is_ok(), "{arg} must be recognised");
        }
        // The space-separated forms consume their operand: it must not be
        // mistaken for an input file (which the driver would try to compile).
        let mut d = Driver::new();
        let args = vec![
            "ccc".to_string(),
            "--sysroot".to_string(),
            "/opt/sysroot".to_string(),
            "x.c".to_string(),
        ];
        d.parse_cli_args(&args).expect("--sysroot <dir>");
        assert_eq!(d.input_files, vec!["x.c".to_string()]);
        // ...and a missing operand is an error, not a silently empty sysroot.
        let mut d = Driver::new();
        let args = vec!["ccc".to_string(), "-isysroot".to_string()];
        assert!(d.parse_cli_args(&args).is_err());
    }

    /// Recognising the GNU link-file switches is not enough: the link layer
    /// only sees `build_linker_args`.  Before they were forwarded both were
    /// silent no-ops (crt1.o/crti.o still linked, so a freestanding `_start`
    /// collided with crt1's and `-shared -nostartfiles` still got crtbeginS).
    #[test]
    fn nostartfiles_and_nodefaultlibs_reach_the_linker() {
        for (flags, want_start, want_libs) in [
            (&["-nostartfiles"][..], true, false),
            (&["-nodefaultlibs"][..], false, true),
            (&["-nostartfiles", "-nodefaultlibs"][..], true, true),
            (&[][..], false, false),
        ] {
            let mut d = Driver::new();
            let mut args = vec!["ccc".to_string()];
            args.extend(flags.iter().map(|f| f.to_string()));
            args.push("x.c".to_string());
            d.parse_cli_args(&args).expect("link-file switches parse");
            let link = d.build_linker_args();
            let has = |f: &str| link.iter().any(|a| a == f);
            assert_eq!(has("-nostartfiles"), want_start, "{flags:?} -> {link:?}");
            assert_eq!(has("-nodefaultlibs"), want_libs, "{flags:?} -> {link:?}");
        }
    }

    /// The `-g` family, pinned against the measured GNU boundary.
    ///
    /// The old `-g*` blanket accepted every one of these silently, so
    /// `-gbogus`/`-gdwarf-9`/`-g4` were reported as implemented features by a
    /// capability probe and the request "select a debug format" produced a
    /// silent lie.  What GNU accepts is tied out in both directions: honoured
    /// selectors build quietly, GNU-accepted-but-unimplemented ones are
    /// diagnosed (and fatal under strict), and GNU-rejected ones fail.
    #[test]
    fn debug_selector_boundary_matches_gnu() {
        for arg in ["-g", "-g1", "-g2", "-g3", "-g0", "-ggdb", "-ggdb3"] {
            assert!(try_flag(arg).is_ok(), "{arg} must be honoured");
        }
        for arg in [
            "-gdwarf",
            "-gdwarf-2",
            "-gdwarf-4",
            "-gdwarf-5",
            "-gdwarf32",
            "-gdwarf64",
            "-gstabs",
            "-gxcoff+",
            "-gcoff3",
            "-gvms",
            "-gcodeview",
            "-gbtf",
            "-gctf",
            "-gz=none",
            "-gz=zlib",
            "-gz=zstd",
            "-grecord-gcc-switches",
            "-gcolumn-info",
            "-gvariable-location-views",
            "-gvariable-location-views=incompat5",
            "-gtoggle",
            "-gno-record-gcc-switches",
            "-gno-column-info",
            "-gno-pubnames",
            "-gno-strict-dwarf",
        ] {
            assert!(try_flag(arg).is_ok(), "{arg} must be tolerated");
            assert!(
                super::unknown_option_tolerated(arg, true).is_err(),
                "{arg}: strict mode must not report it as implemented"
            );
        }
        for arg in [
            "-g4",
            "-g9",
            "-ggdb4",
            "-ggdb9",
            "-gdwarf-1",
            "-gdwarf-6",
            "-gdwarf-9",
            "-gz=bogus",
            "-gno-bogus",
            "-glto",
            "-gbogus",
            "-gnorecord-gcc-switches",
        ] {
            let err = try_flag(arg).expect_err(arg);
            assert!(err.contains(arg), "{err}");
        }
    }

    /// `-g0`/`-ggdb0` keep their GCC meaning: an explicit OFF.
    #[test]
    fn debug_off_selectors_disable_debug_info() {
        for arg in ["-g0", "-ggdb0"] {
            let mut d = Driver::new();
            let args = vec!["ccc".to_string(), arg.to_string(), "x.c".to_string()];
            d.parse_cli_args(&args).expect(arg);
            assert!(!d.debug_info, "{arg} must turn debug info off");
        }
    }

    /// A hardening request lccc cannot honour is diagnosed where it enters
    /// the driver, not silently dropped by the preprocessor setup.
    #[test]
    fn fortify_source_request_is_diagnosed() {
        // Accepted (source compatibility) but never silent: the diagnostic
        // comes from `fortify_source_unsupported`.
        assert!(try_flag("-D_FORTIFY_SOURCE=2").is_ok());
        // Strict mode must not let a probe conclude the surface exists.
        let err = super::fortify_source_unsupported(true)
            .expect_err("strict mode must reject a hardening request it cannot honour");
        assert!(err.contains("_FORTIFY_SOURCE"), "{err}");
        assert!(err.contains("LCCC_STRICT_OPTIONS"), "{err}");
        // The neutralisation is unconditional, so it fires with or without a
        // value and for the `-D`/`-D ` spellings alike.
        assert!(try_flag("-D_FORTIFY_SOURCE").is_ok());
    }

    // ---- -std= dialect contract -----------------------------------------

    /// Every dialect the suggestion table advertises must actually be accepted;
    /// otherwise the "did you mean" hint points at another rejection.
    #[test]
    fn every_advertised_std_dialect_is_accepted() {
        for d in super::STD_DIALECTS {
            let flag = format!("-std={d}");
            assert!(try_flag(&flag).is_ok(), "{flag} must be accepted");
        }
    }

    #[test]
    fn unknown_std_dialect_is_a_hard_error_with_suggestion() {
        let err = try_flag("-std=bogus99").expect_err("-std=bogus99 must be rejected");
        assert!(
            err.contains("unrecognized command-line option '-std=bogus99'"),
            "{err}"
        );

        // A suggestion must always be a dialect LCCC actually accepts:
        // pointing at another rejection would be worse than staying silent.
        for bad in [
            "-std=c17x",
            "-std=gnu1z",
            "-std=c1l",
            "-std=c11z",
            "-std=gnu99y",
        ] {
            let err = try_flag(bad).expect_err(bad);
            let suggested = err
                .split("did you mean '-std=")
                .nth(1)
                .and_then(|s| s.split('\'').next());
            assert!(
                suggested.is_some(),
                "{bad} was rejected without a 'did you mean' hint: {err}"
            );
            let suggested = suggested.unwrap();
            assert!(
                super::STD_DIALECTS.contains(&suggested),
                "{bad} suggested unaccepted dialect {suggested}"
            );
            assert!(
                try_flag(&format!("-std={suggested}")).is_ok(),
                "{bad} suggested {suggested}, which is not accepted"
            );
        }

        // The two cases where a transposition/insertion typo has a unique,
        // obviously-intended target are pinned so a regression in the
        // tie-break is caught rather than merely reproducing itself.
        for (bad, want) in [("-std=c17x", "c17"), ("-std=c1l", "c11")] {
            let err = try_flag(bad).expect_err(bad);
            assert!(
                err.contains(&format!("did you mean '-std={want}'")),
                "{bad} should have suggested {want}, got: {err}"
            );
        }
    }

    /// A `-std=` value that is not close to anything must still be rejected,
    /// just without a possibly-misleading pointer.
    ///
    /// Deliberate divergence from GCC, recorded here so it is a decision and
    /// not an accident: GCC 14/16 answers `-std=bogus99` with
    /// `did you mean '-std=gnu99'?`, i.e. it leaves the `c`/`gnu`/`iso9899:`
    /// family the user was clearly typing in. LCCC suppresses the hint when no
    /// same-family dialect is within [`STD_SUGGEST_MAX_DISTANCE`]; a wrong-but-
    /// confident suggestion is worse than a clean rejection. The rejection
    /// itself -- the part that matters for correctness -- matches GCC exactly.
    #[test]
    fn distant_std_dialect_is_rejected_without_a_guess() {
        let err = try_flag("-std=qwertyuiop").expect_err("-std=qwertyuiop must be rejected");
        assert!(
            err.contains("unrecognized command-line option '-std=qwertyuiop'"),
            "{err}"
        );
        assert!(
            !err.contains("did you mean"),
            "no near neighbour exists: {err}"
        );
    }

    /// The suggestion must not fire when nothing is remotely close, and must
    /// never cross the c/gnu/iso prefix boundary.
    #[test]
    fn std_suggestion_is_conservative() {
        assert_eq!(super::closest_std_dialect("zzzzzzzzzz"), None);
        assert_eq!(super::closest_std_dialect(""), None);
        for probe in ["c17x", "gnu1z", "c99", "gnu23", "iso9899:2017"] {
            if let Some(s) = super::closest_std_dialect(probe) {
                assert_eq!(
                    s.as_bytes().first(),
                    probe.as_bytes().first(),
                    "{probe} -> {s} crossed the dialect prefix"
                );
            }
        }
    }

    // ---- contract flags that must never be silently dropped -------------

    /// `-fstack-protector*` is the established precedent; these two requests
    /// have the same invisible-absence failure mode and must be refused too.
    #[test]
    fn instrumentation_requests_are_refused_not_ignored() {
        for f in [
            "-ftrapv",
            "-fsanitize=address",
            "-fsanitize=undefined",
            "-fsanitize",
        ] {
            assert!(
                try_flag(f).is_err(),
                "{f} must be refused, not silently ignored"
            );
        }
        // The matching *disable* requests stay accepted: a compiler that never
        // emits the instrumentation already complies.
        for f in ["-fno-trapv", "-fsanitize-recover=all"] {
            assert!(try_flag(f).is_ok(), "{f} must be accepted");
        }
    }

    // ---- -ffreestanding (C11 4p6) ----------------------------------------

    /// Both spellings must be accepted, and `-fhosted` must undo
    /// `-ffreestanding` (GCC applies them last-wins), rather than the pair
    /// being an unknown-option diagnostic.
    #[test]
    fn freestanding_and_hosted_are_accepted() {
        for f in ["-ffreestanding", "-fhosted"] {
            assert!(try_flag(f).is_ok(), "{f} must be accepted");
        }
    }

    #[test]
    fn edit_distance_matches_known_values() {
        assert_eq!(super::edit_distance("", ""), 0);
        assert_eq!(super::edit_distance("c17", "c17"), 0);
        assert_eq!(super::edit_distance("c17x", "c17"), 1);
        assert_eq!(super::edit_distance("", "abc"), 3);
        assert_eq!(super::edit_distance("abc", ""), 3);
        assert_eq!(super::edit_distance("kitten", "sitting"), 3);
        // "gnu17" vs "gnu23": the two trailing digits both differ.
        assert_eq!(super::edit_distance("gnu17", "gnu23"), 2);
        assert_eq!(super::common_prefix_len("c17x", "c17"), 3);
        assert_eq!(super::common_prefix_len("c17x", "c1x"), 2);
        assert_eq!(super::common_prefix_len("abc", "abd"), 2);
        assert_eq!(super::common_prefix_len("", "abc"), 0);
    }
}

#[cfg(test)]
mod prefetch_isa_tests {
    //! Prefetch-ISA resolution (defect 14 and PRFCHW). Every expectation is
    //! GCC 16.2's measured behaviour for the same command line (`gcc-16 -O2
    //! -S` over all eight `__builtin_prefetch(p, rw, loc)` forms).
    use crate::driver::Driver;

    fn parse(flags: &[&str]) -> Driver {
        let mut d = Driver::new();
        let args: Vec<String> = std::iter::once("ccc")
            .chain(flags.iter().copied())
            .chain(std::iter::once("x.c"))
            .map(str::to_string)
            .collect();
        assert!(d.parse_cli_args(&args).is_ok(), "{flags:?}");
        d
    }

    #[test]
    fn i686_sse_prefetch_needs_sse_or_prfchw() {
        // The i386 default baseline is the x86-64 ISA level, so the
        // prefetcht*/nta family is baseline for plain `-m32` (GCC 16.2 and
        // Clang 23.1.3 both emit it there, measured). An explicit pre-SSE
        // `-march=` still suppresses it (both oracles: 0 instructions for
        // `-m32 -march=i686`), as does `-mno-sse`.
        //
        // The `-mno-sse` corner is a measured oracle fork: GCC 16.2 EMITS
        // `prefetcht0` under `-m32 -mno-sse` (0F 18, an SSE encoding), while
        // Clang 23.1.3 suppresses it. lccc follows Clang: the `-mno-sse`
        // contract is that NO SSE instruction of any encoding is emitted
        // (the kernel's -mno-sse use is precisely to avoid FPU/SSE state),
        // and a lone prefetch hint does not justify breaking that invariant.
        for flags in [
            &["-m32", "-march=i686"][..],
            &["-m32", "-march=pentiumpro"],
            &["-m32", "-msse", "-mno-sse"],
            &["-m32", "-mno-sse"],
            &["-m32", "-march=raptorlake", "-mno-sse", "-mno-prfchw"],
        ] {
            assert!(!parse(flags).resolved_prefetch_sse(), "{flags:?}");
        }
        // The prefetcht*/nta family is granted for these.
        for flags in [
            &["-m32"][..],
            &["-m32", "-msse"],
            &["-m32", "-msse2"],
            &["-m32", "-msse", "-march=i686"],
            &["-m32", "-march=i686", "-msse"],
            &["-m32", "-msse4.1"],
            &["-m32", "-march=nehalem"],
            &["-m32", "-march=raptorlake"],
            &["-m32", "-mprfchw"],
            &["-m32", "-mno-sse", "-mprfchw"],
        ] {
            assert!(parse(flags).resolved_prefetch_sse(), "{flags:?}");
        }
    }

    #[test]
    fn legacy_sse41_permission_is_not_the_x86_64_codegen_ceiling() {
        use crate::common::x86_features::X86Feature as F;
        for mode in ["-m32", "-m64"] {
            for (flags, granted) in [
                (&[][..], false),
                (&["-msse4.1"][..], true),
                (&["-march=nehalem"][..], true),
                (&["-msse4.1", "-mno-sse4.1"][..], false),
                (&["-mno-sse4.1", "-msse4.1"][..], true),
                (&["-msse4.1", "-mno-sse2", "-msse"][..], false),
                (&["-msse4.1", "-mno-sse"][..], false),
            ] {
                let mut args = vec![mode];
                args.extend_from_slice(flags);
                assert_eq!(
                    parse(&args).effective_x86_features().contains(F::Sse41),
                    granted,
                    "{args:?}"
                );
            }
        }
    }

    #[test]
    fn i686_sse_feature_mirror_tracks_abi_permission() {
        // The i386 default baseline is the x86-64 ISA level: `-m32` alone
        // resolves SSE AND SSE2 on (GCC 16.2 default `-m32` = `-march=x86-64`
        // with both [enabled], Clang 23.1.3 agrees). Only an explicit
        // pre-SSE `-march=`, the sticky `-mno-sse`, or (SSE2 only) the
        // sticky `-mno-sse2` / `-mgeneral-regs-only` denial drop the level;
        // explicit grants override the `-march=` veto and lift the
        // `-mno-sse2` denial (last-explicit wins, in either order). The
        // effective-feature mirror and the resolved i686 levels must agree.
        for (flags, sse, sse2) in [
            // (flags, resolved SSE level, resolved SSE2 level)
            (&["-m32"][..], true, true),
            (&["-m32", "-march=i686"][..], false, false),
            (&["-m32", "-march=i386"][..], false, false),
            (&["-m32", "-march=i686", "-msse"][..], true, false),
            (&["-m32", "-march=i686", "-msse2"][..], true, true),
            (&["-m32", "-msse", "-march=i686"][..], true, false),
            (&["-m32", "-msse"][..], true, true),
            (&["-m32", "-msse2"][..], true, true),
            (&["-m32", "-msse4.1"][..], true, true),
            (&["-m32", "-march=nehalem"][..], true, true),
            (&["-m32", "-march=i686", "-march=nehalem"][..], true, true),
            (&["-m32", "-march=nehalem", "-march=i686"][..], false, false),
            (&["-m32", "-msse", "-mno-sse"][..], false, false),
            (&["-m32", "-mno-sse", "-msse"][..], true, true),
            (&["-mno-sse", "-msse", "-m32"][..], true, true),
            (&["-m32", "-mno-sse"][..], false, false),
            (&["-m32", "-mno-sse2"][..], true, false),
            (&["-mno-sse2", "-m32"][..], true, false),
            (&["-m32", "-msse", "-mno-sse2"][..], true, false),
            (&["-m32", "-mno-sse2", "-msse"][..], true, false),
            (&["-msse", "-mno-sse2", "-m32"][..], true, false),
            (&["-m32", "-mno-sse2", "-msse2"][..], true, true),
            (&["-m32", "-mno-sse", "-mno-sse2", "-msse"][..], true, false),
            (&["-m32", "-mno-sse2", "-mno-sse", "-msse"][..], true, false),
            (&["-m32", "-mgeneral-regs-only"][..], false, false),
            (&["-m32", "-mgeneral-regs-only", "-msse"][..], true, false),
            (&["-m32", "-mgeneral-regs-only", "-msse2"][..], true, true),
            (&["-m32", "-msse", "-mgeneral-regs-only"][..], false, false),
            (
                &["-m32", "-mgeneral-regs-only", "-msse", "-march=nehalem"][..],
                true,
                false,
            ),
            (
                &["-m32", "-msse", "-mgeneral-regs-only", "-march=nehalem"][..],
                false,
                false,
            ),
            (
                &["-m32", "-mgeneral-regs-only", "-msse2", "-march=nehalem"][..],
                true,
                true,
            ),
            (
                &["-m32", "-msse2", "-mgeneral-regs-only", "-march=nehalem"][..],
                false,
                false,
            ),
            (&["-m32", "-msse2", "-mno-sse2"][..], true, false),
            (&["-m32", "-msse2", "-msse"][..], true, true),
            (&["-m32", "-msse", "-msse2"][..], true, true),
            (&["-m32", "-mno-sse", "-msse2"][..], true, true),
            (&["-m32", "-mno-sse2", "-msse3"][..], true, true),
            (&["-m32", "-mno-sse2", "-mavx"][..], true, true),
            (&["-m32", "-mno-sse", "-mavx"][..], true, true),
            (&["-m32", "-mno-sse2", "-march=nehalem"][..], true, false),
            (&["-m32", "-mno-sse2", "-maes"][..], true, true),
            (&["-m32", "-mno-sse2", "-mpclmul"][..], true, true),
            (&["-m32", "-mno-sse2", "-mgfni"][..], true, false),
            (&["-m32", "-mno-sse2", "-mavxifma"][..], true, true),
            (&["-m32", "-mno-sse2", "-mavxneconvert"][..], true, true),
            (&["-m32", "-mno-sse2", "-msha512"][..], true, true),
            (&["-m32", "-mno-sse2", "-msm3"][..], true, true),
            (&["-m32", "-mno-sse2", "-msm4"][..], true, true),
            (&["-m32", "-mno-sse2", "-mmovrs"][..], true, false),
            (&["-m32", "-mno-sse2", "-mamx-tile"][..], true, false),
            (&["-m32", "-mno-sse2", "-mcmpccxadd"][..], true, false),
        ] {
            let driver = parse(flags);
            assert_eq!(driver.resolved_i686_sse(), sse, "{flags:?}");
            assert_eq!(driver.resolved_i686_sse2(), sse2, "{flags:?}");
            let features = driver.effective_x86_features();
            assert_eq!(
                features.contains(crate::common::x86_features::X86Feature::Sse),
                sse,
                "effective SSE mirror disagrees for {flags:?}"
            );
            assert_eq!(
                features.contains(crate::common::x86_features::X86Feature::Sse2),
                sse2,
                "effective SSE2 mirror disagrees for {flags:?}"
            );
        }

        // `-mgeneral-regs-only` is a full SSE denial (it sets `no_sse`, so
        // the resolved levels drop too): no XMM state of any kind, which
        // also vetoes the published features and the prefetch grant.
        let driver = parse(&["-m32", "-mgeneral-regs-only"]);
        assert!(!driver.resolved_i686_sse());
        assert!(!driver.resolved_i686_sse2());
        assert!(
            !driver
                .effective_x86_features()
                .contains(crate::common::x86_features::X86Feature::Sse)
        );
        assert!(
            !driver
                .effective_x86_features()
                .contains(crate::common::x86_features::X86Feature::Sse2)
        );
        assert!(!driver.resolved_prefetch_sse());
    }

    #[test]
    fn i686_native_avx_permissions_keep_vectorizer_ceiling_closed() {
        use crate::common::x86_features::X86Feature as F;

        for (flags, avx, avx2) in [
            (&["-m32"][..], false, false),
            (&["-m32", "-mavx"][..], true, false),
            (&["-m32", "-mavx2"][..], true, true),
            (&["-m32", "-mavx2", "-mno-avx2"][..], true, false),
            (&["-m32", "-mavx", "-mno-avx"][..], false, false),
            (&["-m32", "-mno-avx", "-mavx"][..], true, false),
            (&["-m32", "-mavx2", "-mno-avx2", "-mavx2"][..], true, true),
            (&["-m32", "-mavx2", "-mno-sse2"][..], false, false),
            (&["-m32", "-mavx2", "-mno-sse"][..], false, false),
            (&["-m32", "-mavx2", "-mgeneral-regs-only"][..], false, false),
        ] {
            let driver = parse(flags);
            let features = driver.effective_x86_features();
            assert_eq!(features.contains(F::Avx), avx, "{flags:?}");
            assert_eq!(features.contains(F::Avx2), avx2, "{flags:?}");
            assert!(!driver.x86_isa().avx, "{flags:?}");
            assert!(!driver.x86_isa().ymm, "{flags:?}");
        }
    }

    #[test]
    fn x86_64_sse_prefetch_is_baseline() {
        // GCC 16 emits prefetcht* even without xmm state on x86-64.
        for flags in [
            &[][..],
            &["-mno-sse"],
            &["-mno-sse2"],
            &["-mgeneral-regs-only"],
        ] {
            assert!(parse(flags).resolved_prefetch_sse(), "{flags:?}");
        }
    }

    #[test]
    fn prfchw_follows_march_and_explicit_flags() {
        for flags in [
            &[][..],
            &["-march=x86-64-v3"],
            &["-march=x86-64-v4"],
            &["-march=haswell"],
            &["-march=nehalem"],
            &["-march=raptorlake", "-mno-prfchw"],
            &["-mno-prfchw", "-march=raptorlake"],
            &["-mprfchw", "-mno-prfchw"],
            &["-mprfchw", "-march=raptorlake", "-mno-prfchw"],
            &["-march=raptorlake", "-march=haswell"],
        ] {
            assert!(!parse(flags).resolved_prfchw(), "{flags:?}");
        }
        for flags in [
            &["-mprfchw"][..],
            &["-mno-prfchw", "-mprfchw"],
            &["-mprfchw", "-march=haswell"],
            &["-march=haswell", "-mprfchw"],
            &["-march=broadwell"],
            &["-march=raptorlake"],
            &["-march=znver4"],
            &["-march=silvermont"],
            &["-march=sapphirerapids"],
            &["-m32", "-march=raptorlake"],
        ] {
            assert!(parse(flags).resolved_prfchw(), "{flags:?}");
        }
    }

    #[test]
    fn march_table_matches_gcc_prfchw_set() {
        // `gcc-16 -march=X -dM -E | grep -c __PRFCHW__`, measured.
        for m in [
            "nehalem",
            "westmere",
            "sandybridge",
            "ivybridge",
            "haswell",
            "x86-64",
            "x86-64-v2",
            "x86-64-v3",
            "x86-64-v4",
            "generic",
            "i686",
        ] {
            assert!(!super::x86_march_implies_prfchw(m), "{m}");
        }
        for m in [
            "silvermont",
            "goldmont",
            "broadwell",
            "skylake",
            "znver1",
            "znver5",
            "alderlake",
            "raptorlake",
            "meteorlake",
            "arrowlake-s",
            "lunarlake",
            "skylake-avx512",
            "icelake-server",
            "sapphirerapids",
            "graniterapids-d",
            "clearwaterforest",
        ] {
            assert!(super::x86_march_implies_prfchw(m), "{m}");
        }
    }
}
