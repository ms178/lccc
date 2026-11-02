//! x86-64 code-generation ISA permission — the single source of truth.
//!
//! # Why one struct
//!
//! Before this module the x86 backend derived "may I emit this encoding?"
//! from four unrelated places: the driver's `enable_*` request bits (what
//! the TU *asked for*, which drive the `__AVX2__`-style macros), the
//! vectoriser's thread-locals (`set_x86_simd_isa`), `X86Codegen::{no_sse,
//! avx2_enabled, fma_enabled}`, and — for most scalar FP — nothing at all:
//! `float_ops.rs`, `machinst_emit.rs` and `comparison.rs` spelled every
//! scalar FP op in its VEX form unconditionally.  Measured on the regression
//! corpus at `-O2 -mno-avx` before the fix: 2 400+ VEX instructions
//! (`vaddsd` 473, `vmulsd` 453, `vmovss` 369, `vmovsd` 346, …), 74 `pmulld`
//! (SSE4.1) and 8 `%ymm` references from the "SSE2" matmul lowering.  Every
//! one of those is `#UD` on a machine without AVX, and the Linux kernel
//! (CR4.OSXSAVE=0, `-mno-avx`) is exactly such a machine even on AVX
//! hardware.
//!
//! # Policy
//!
//! * **No `-march`** — LCCC's documented code-generation baseline is
//!   x86-64 (SSE2 + the x86-64 integer ISA, no POPCNT/BMI/AVX).  Post-x86-64
//!   classes are opt-in: an explicit `-march=` (e.g. `-march=x86-64-v3`) or the
//!   matching `-m*` flag grants them.  Every measured benchmark
//!   number in `engineering/` depends on that, so a default-flag build keeps
//!   every subset *unless the TU explicitly denied it* (`-mno-avx`,
//!   `-mno-sse4.1`, `-mno-fma`, `-mno-sse`, `-mgeneral-regs-only`).
//! * **Explicit `-march=<level|cpu|native>`** — GCC-exact: the profile is the
//!   ceiling.  `-march=x86-64` / `x86-64-v2` therefore means SSE2 / SSE4.2
//!   *only*; emitting AVX2 into an object the user pinned to v2 would SIGILL
//!   on the hardware they named.  Absent-flag and explicit-flag builds now
//!   differ only in the default, which is the one place the project
//!   consciously deviates from GCC.
//! * **`-mno-<feat>` is sticky against `-march`** (GCC applies explicit
//!   target bits over the arch defaults regardless of order); a later
//!   explicit `-m<feat>` re-enables it (last explicit flag wins).
//!
//! # Per-function permission
//!
//! [`X86Isa::with_target_features`] narrows or widens the TU's ceiling for
//! one `__attribute__((target(...)))` function, with GCC 16.2's measured
//! semantics: the last `arch=` replaces the TU's ISA and discards the
//! options before it, a CPU's ISA comes from the generated GCC table
//! ([`X86Isa::arch_row`], `arch_table.rs`), and the other options apply in
//! order through GCC's measured masks ([`function_feature_bits`]:
//! `no-sse4.2` drops AVX, a later `fma` grants it back). A CPU GCC rejects
//! is diagnosed and gets the x86-64 baseline (on i686, nothing); AVX-512
//! CPUs grant exactly [`X86Isa::V3`], since lccc emits no EVEX code.
//! `docs/x86-isa-permissions-and-simd-abi.md` has the whole model.
//!
//! # Consumers
//!
//! * `passes::run_passes` (vectoriser width/FMA/SSE4.1 gates, the
//!   `fma()`/`floor()` libcall folds).
//! * `X86Codegen::apply_options` → `isa` field (text emitters).
//! * The process-global mirror ([`current`]) for code that has no `&self`:
//!   `machinst_emit` and the peephole passes.  Compilation of one TU is
//!   single-threaded and `apply_options` refreshes the global per TU, the
//!   same discipline as `isel::set_sse_integer_moves` and
//!   `simplify::set_has_fma3`.

use crate::common::x86_features::{EffectiveFeatures, X86Feature};
use std::sync::atomic::{AtomicU8, Ordering};

/// Legal x86-64 encodings for the current translation unit.
///
/// The fields are monotone: `ymm ⇒ avx ⇒ sse41 ⇒ simd` and `fma ⇒ avx`.
/// [`X86Isa::normalized`] enforces that so a caller can never assert FMA3
/// while denying VEX.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct X86Isa {
    /// SSE2 vector-codegen permission (`-mno-sse`, `-mno-sse2` and
    /// `-mgeneral-regs-only` clear it). A later independent SSE1 grant does
    /// not reopen this ceiling; SSE1 F32 maps have separate typed permission.
    pub simd: bool,
    /// SSE4.1 (`pmulld`, `roundsd`, `blendvpd`, `pmovsx*`, `pinsr*` …).
    pub sse41: bool,
    /// VEX encoding is legal (AVX).  Governs every `v*` mnemonic, including
    /// the 128-bit scalar forms `vaddsd` / `vmovsd` / `vroundsd`.
    pub avx: bool,
    /// 256-bit `ymm` forms (AVX2, including the AVX2 integer set).
    pub ymm: bool,
    /// FMA3 (`vfmadd*`), VEX-encoded.
    pub fma: bool,
    /// MOVBE (`movbe`) is legal.  Deliberately OUTSIDE the SSE implication
    /// chain above: it is an integer load/store that performs a byte swap, and
    /// it is not implied by — nor does it imply — any SIMD feature.  Its
    /// permission comes from `-mmovbe`/`-mno-movbe` and the `-march=` baseline
    /// (`x86-64-v3` and the named micro-architectures include it, plain
    /// `x86-64` does not), mirroring `Driver::resolved_movbe`, which is also
    /// where `__builtin_cpu_supports("movbe")` gets its answer.
    pub movbe: bool,
    /// CMPXCHG16B (`lock cmpxchg16b`) is legal.  Integer ISA like MOVBE and
    /// granted only by `-mcx16` or a v2+ `-march` profile (GCC-exact).  When
    /// it is off, 16-byte atomics call libatomic instead.
    pub cx16: bool,
}

impl X86Isa {
    /// Nothing legal — i686, AArch64, RISC-V and `-mgeneral-regs-only`.
    pub const NONE: X86Isa = X86Isa {
        simd: false,
        sse41: false,
        avx: false,
        ymm: false,
        fma: false,
        movbe: false,
        cx16: false,
    };

    /// Plain x86-64 baseline: SSE2 only.
    pub const SSE2: X86Isa = X86Isa {
        simd: true,
        sse41: false,
        avx: false,
        ymm: false,
        fma: false,
        movbe: false,
        cx16: false,
    };

    /// x86-64-v3: granted only by an explicit `-march=x86-64-v3` (or a
    /// superset profile); never the default.
    pub const V3: X86Isa = X86Isa {
        simd: true,
        sse41: true,
        avx: true,
        ymm: true,
        fma: true,
        movbe: true,
        cx16: true,
    };

    /// Enforce the implication chain (`ymm ⇒ avx ⇒ sse41 ⇒ simd`, `fma ⇒ avx`).
    #[must_use]
    pub const fn normalized(self) -> X86Isa {
        let simd = self.simd;
        let sse41 = self.sse41 && simd;
        let avx = self.avx && sse41;
        let ymm = self.ymm && avx;
        let fma = self.fma && avx;
        X86Isa {
            simd,
            sse41,
            avx,
            ymm,
            fma,
            movbe: self.movbe,
            cx16: self.cx16,
        }
    }

    /// The permission of a feature set (an `EffectiveFeatures` bitmask):
    /// each class is its defining feature, with the implication chain
    /// enforced (AVX2 without SSE2 is no 256-bit class).
    #[must_use]
    pub const fn from_feature_bits(bits: u64) -> X86Isa {
        const fn has(bits: u64, feature: X86Feature) -> bool {
            bits & (1 << feature as u8) != 0
        }
        X86Isa {
            simd: has(bits, X86Feature::Sse2),
            sse41: has(bits, X86Feature::Sse41),
            avx: has(bits, X86Feature::Avx),
            ymm: has(bits, X86Feature::Avx2),
            fma: has(bits, X86Feature::Fma),
            movbe: has(bits, X86Feature::Movbe),
            cx16: has(bits, X86Feature::Cx16),
        }
        .normalized()
    }

    /// `bits` with each class set to this permission's choice: the start of
    /// a function's fold when the module permission is narrower than the
    /// TU's features (a `CCC_NO_*` knob) or there is no driver (unit tests).
    #[must_use]
    pub fn overlay(self, bits: u64) -> u64 {
        use X86Feature as F;
        if Self::from_feature_bits(bits) == self {
            return bits;
        }
        [
            (self.simd, &[F::Sse, F::Sse2][..]),
            (self.sse41, &[F::Sse3, F::Ssse3, F::Sse41]),
            (self.avx, &[F::Avx]),
            (self.ymm, &[F::Avx2]),
            (self.fma, &[F::Fma]),
            (self.movbe, &[F::Movbe]),
            (self.cx16, &[F::Cx16]),
        ]
        .into_iter()
        .fold(bits, |bits, (on, features)| {
            features
                .iter()
                .fold(bits, |bits, &feature| with_feature(bits, feature, on))
        })
    }

    /// This module permission for a function with the `target("...")`
    /// options `features` ([`function_feature_bits`], GCC's semantics).
    #[must_use]
    pub fn with_target_features(self, features: &[String]) -> X86Isa {
        if features.is_empty() {
            return self;
        }
        let tu = crate::common::x86_features::effective().raw();
        Self::from_feature_bits(fold_target_features(self.overlay(tu), features))
    }

    /// GCC 16.2's row for an `arch=` CPU name, or `None` for a name GCC
    /// rejects (`generic` and `native` included).
    #[must_use]
    pub fn arch_row(arch: &str) -> Option<&'static super::arch_table::X86ArchRow> {
        super::arch_table::x86_arch_row(arch)
    }

    /// The permission legal in BOTH `self` and `other` (field-wise AND): the
    /// ISA under which text mixing functions of either permission may be
    /// rewritten.
    #[must_use]
    pub const fn meet(self, other: X86Isa) -> X86Isa {
        X86Isa {
            simd: self.simd && other.simd,
            sse41: self.sse41 && other.sse41,
            avx: self.avx && other.avx,
            ymm: self.ymm && other.ymm,
            fma: self.fma && other.fma,
            movbe: self.movbe && other.movbe,
            cx16: self.cx16 && other.cx16,
        }
        .normalized()
    }

    const BIT_SIMD: u8 = 1 << 0;
    const BIT_SSE41: u8 = 1 << 1;
    const BIT_AVX: u8 = 1 << 2;
    const BIT_YMM: u8 = 1 << 3;
    const BIT_FMA: u8 = 1 << 4;
    const BIT_MOVBE: u8 = 1 << 5;
    const BIT_CX16: u8 = 1 << 6;

    const fn to_bits(self) -> u8 {
        let n = self.normalized();
        (if n.simd { Self::BIT_SIMD } else { 0 })
            | (if n.sse41 { Self::BIT_SSE41 } else { 0 })
            | (if n.avx { Self::BIT_AVX } else { 0 })
            | (if n.ymm { Self::BIT_YMM } else { 0 })
            | (if n.fma { Self::BIT_FMA } else { 0 })
            | (if n.movbe { Self::BIT_MOVBE } else { 0 })
            | (if n.cx16 { Self::BIT_CX16 } else { 0 })
    }

    const fn from_bits(b: u8) -> X86Isa {
        X86Isa {
            simd: b & Self::BIT_SIMD != 0,
            sse41: b & Self::BIT_SSE41 != 0,
            avx: b & Self::BIT_AVX != 0,
            ymm: b & Self::BIT_YMM != 0,
            fma: b & Self::BIT_FMA != 0,
            movbe: b & Self::BIT_MOVBE != 0,
            cx16: b & Self::BIT_CX16 != 0,
        }
    }
}

impl Default for X86Isa {
    /// The x86-64 baseline (SSE2, no VEX).  Unit tests and isolated emitters
    /// that never go through the driver see this baseline; anything that needs
    /// a post-x86-64 class must request it explicitly.
    fn default() -> Self {
        X86Isa::SSE2
    }
}

/// Process-global mirror of the current TU's permission for emitters that
/// have no access to `X86Codegen` (MachInst emission, peephole passes).
static CURRENT: AtomicU8 = AtomicU8::new(X86Isa::SSE2.to_bits());

/// Publish `isa` for the code paths without `&self`.  Called by
/// `X86Codegen::apply_options` once per translation unit.
pub fn set_current(isa: X86Isa) {
    CURRENT.store(isa.to_bits(), Ordering::Relaxed);
}

/// The permission published by the last [`set_current`]; the x86-64 baseline
/// (SSE2) until a driver publishes one.
#[inline]
pub fn current() -> X86Isa {
    X86Isa::from_bits(CURRENT.load(Ordering::Relaxed))
}

/// `bits` with `feature` present iff `on`.
#[must_use]
pub const fn with_feature(bits: u64, feature: X86Feature, on: bool) -> u64 {
    let bit = 1 << feature as u8;
    if on { bits | bit } else { bits & !bit }
}

/// The features of a function whose `target("...")` options are
/// `features`, in the translation unit whose features are `tu`, of which
/// the command line set or cleared `tu_explicit` explicitly. GCC 16.2's
/// semantics, measured into [`super::arch_table`]:
///
/// * The options apply in order, each a measured mask: `NAME` clears and
///   sets, `no-NAME` unsets, each with everything GCC's implication graph
///   attaches (`avx512bmm` sets AVX512BW and AVX2, `no-avx2` unsets every
///   AVX-512 subset, `general-regs-only` clears the SSE register class and
///   keeps BMI2, POPCNT, AES). `avx512bmm,no-avx512bmm` therefore keeps
///   AVX512BW, as in GCC. The list must keep the attribute's order.
/// * `arch=CPU` replaces the features with the CPU's and forgets what was
///   explicit: under `-mpopcnt`, `target("arch=x86-64")` has no POPCNT.
///   An option after it that removes ABM also removes what the CPU has
///   only through ABM, unless an option named it: `arch=amdfam10,no-abm`
///   has neither POPCNT nor LZCNT, `arch=amdfam10,popcnt,no-abm` POPCNT
///   ([`super::arch_table::X86ArchRow::abm_derived`]).
/// * Then GCC's override implications ([`super::arch_table::X86_IMPLIED`]):
///   SSE4.2 brings POPCNT, SSE brings MMX and ABM brings LZCNT, unless an
///   option (or the command line, before an `arch=`) named the implied
///   feature: `-mno-popcnt` with `target("sse4.2")` has no POPCNT.
/// * `tune=` and the options lccc does not model (`sha`, `rtm`, ...) change
///   nothing here; a CPU name GCC rejects is diagnosed where the attribute
///   is lowered and gets the x86-64 baseline (on i686, nothing), as does
///   `no-general-regs-only`, which GCC refuses.
#[must_use]
pub fn function_feature_bits(tu: u64, tu_explicit: u64, features: &[String], x86_64: bool) -> u64 {
    use super::arch_table::{ABM, X86_IMPLIED, x86_target_option};
    let (mut bits, mut explicit, mut abm_derived) = (tu, tu_explicit, 0);
    for option in features {
        if let Some(arch) = option.strip_prefix("arch=") {
            let row = X86Isa::arch_row(arch).filter(|row| row.x86_64 || !x86_64);
            (bits, abm_derived) = match row {
                Some(row) => (row.feature_bits(), row.abm_derived()),
                None if x86_64 => (
                    X86Isa::arch_row("x86-64").map_or(0, |row| row.feature_bits()),
                    0,
                ),
                None => (0, 0),
            };
            explicit = 0;
            continue;
        }
        let (negated, name) = match option.strip_prefix("no-") {
            Some(name) => (true, name),
            None => (false, option.as_str()),
        };
        let Some(handler) = x86_target_option(name) else {
            continue;
        };
        if let Some(next) = handler.apply(bits, negated) {
            explicit |= handler.touched(negated);
            bits = if bits & !next & ABM != 0 {
                next & !(abm_derived & !explicit)
            } else {
                next
            };
        }
    }
    for &(trigger, implied) in X86_IMPLIED {
        if bits & trigger != 0 && explicit & implied == 0 {
            bits |= implied;
        }
    }
    bits
}

/// The option of `features` that removed `feature` from the function, if
/// the function lacks it although the translation unit (`tu`) or an
/// earlier option had it: the culprit a diagnostic names.
#[must_use]
pub fn target_option_denying<'a>(
    tu: u64,
    tu_explicit: u64,
    features: &'a [String],
    x86_64: bool,
    feature: X86Feature,
) -> Option<&'a str> {
    let has = |options: &[String]| {
        function_feature_bits(tu, tu_explicit, options, x86_64) & (1 << feature as u8) != 0
    };
    if has(features) {
        return None;
    }
    (0..features.len())
        .rev()
        .find(|&i| has(&features[..i]) && !has(&features[..=i]))
        .map(|i| features[i].as_str())
}

/// [`function_feature_bits`] from `base` in this compilation (the driver's
/// published explicit mask), for x86-64 code.
#[must_use]
pub fn fold_target_features(base: u64, features: &[String]) -> u64 {
    function_feature_bits(
        base,
        crate::common::x86_features::explicit(),
        features,
        true,
    )
}

/// The features of the function being compiled with the `target("...")`
/// options `features`: the translation unit's when there are none.
#[must_use]
pub fn function_features(features: &[String], x86_64: bool) -> EffectiveFeatures {
    use crate::common::x86_features::{effective, explicit};
    if features.is_empty() {
        return effective();
    }
    EffectiveFeatures::from_raw(function_feature_bits(
        effective().raw(),
        explicit(),
        features,
        x86_64,
    ))
}

/// Whether code with `features` may use the intrinsics of `feature`.
/// GCC's builtin masks add one conjunct the feature graph lacks: AES,
/// PCLMUL and GFNI intrinsics also need SSE2, which `no-sse2` removes
/// without removing them (GCC 16.2 rejects `_mm_aesenc_si128` under
/// `target("aes,no-sse2")` and accepts it under `target("no-sse2,aes")`,
/// where `aes` grants SSE2 back: measured).
#[must_use]
pub fn features_permit(features: EffectiveFeatures, feature: X86Feature) -> bool {
    features.contains(feature)
        && (!matches!(
            feature,
            X86Feature::Aes | X86Feature::Pclmul | X86Feature::Gfni
        ) || features.contains(X86Feature::Sse2))
}

/// Whether a function with the `target("...")` options `features` may use
/// the intrinsics of `feature` ([`function_features`], [`features_permit`]).
#[must_use]
pub fn function_feature_permitted(features: &[String], feature: X86Feature, x86_64: bool) -> bool {
    features_permit(function_features(features, x86_64), feature)
}

/// The `target(...)` option that takes the x87 FPU from a function, if
/// any. x87 is no ISA feature bit: the last of `general-regs-only`,
/// `no-80387` and `80387` decides.
#[must_use]
pub fn function_x87_denial(features: &[String]) -> Option<&String> {
    features
        .iter()
        .rev()
        .find(|f| matches!(f.as_str(), "general-regs-only" | "no-80387" | "80387"))
        .filter(|f| *f != "80387")
}

/// Whether a function may use the 128/256-bit dot products (`vpdpbusd`,
/// `vpdpbusds`, `vpdpwssd`, `vpdpwssds`) through their EVEX encoding:
/// AVX512-VNNI with AVX512-VL (GCC 16.2 accepts `_mm256_dpbusd_epi32` under
/// `-mavx512vnni -mavx512vl` and `-march=cascadelake` and emits the EVEX
/// form), on the 256-bit class lccc's emitters need. x86-64 only: the
/// i686 backend has no EVEX encoder.
#[must_use]
pub fn function_evex_vnni_permitted(features: &[String], x86_64: bool) -> bool {
    let set = function_features(features, x86_64);
    x86_64
        && set.contains(X86Feature::Avx512Vnni)
        && set.contains(X86Feature::Avx512Vl)
        && set.contains(X86Feature::Avx2)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feats(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn arch_target_entries_replace_the_baseline() {
        // GCC 16.2 compiles `target("arch=x86-64")` under `-march=haswell`
        // to SSE2 only: the entry replaces, it does not add.
        assert_eq!(
            X86Isa::V3.with_target_features(&feats(&["arch=x86-64"])),
            X86Isa::SSE2
        );
        let v2 = X86Isa::V3.with_target_features(&feats(&["arch=x86-64-v2"]));
        assert!(v2.simd && v2.sse41 && v2.cx16 && !v2.avx && !v2.movbe);
        let snb = X86Isa::SSE2.with_target_features(&feats(&["arch=sandybridge"]));
        assert!(snb.avx && !snb.ymm && !snb.fma);
        assert_eq!(
            X86Isa::SSE2.with_target_features(&feats(&["arch=znver5"])),
            X86Isa::V3
        );
        // Every CPU GCC knows has its measured row: Core 2 has CMPXCHG16B
        // but no SSE4.1; Silvermont has MOVBE without AVX; Piledriver has
        // FMA3 without AVX2.
        assert_eq!(
            X86Isa::V3.with_target_features(&feats(&["arch=core2"])),
            X86Isa {
                cx16: true,
                ..X86Isa::SSE2
            }
        );
        let slm = X86Isa::SSE2.with_target_features(&feats(&["arch=silvermont"]));
        assert!(slm.sse41 && slm.movbe && !slm.avx);
        let pd = X86Isa::SSE2.with_target_features(&feats(&["arch=bdver2"]));
        assert!(pd.fma && pd.avx && !pd.ymm);
        // A name GCC rejects (or a 32-bit-only CPU) never widens: x86-64
        // baseline. Lowering diagnoses both before codegen sees them.
        for bogus in ["arch=generic", "arch=native", "arch=knl", "arch=pentium4"] {
            assert_eq!(
                X86Isa::V3.with_target_features(&feats(&[bogus])),
                X86Isa::SSE2,
                "{bogus}"
            );
        }
        // Grants and denials in the same attribute apply on top of the CPU.
        let grant = X86Isa::V3.with_target_features(&feats(&["arch=x86-64", "avx2"]));
        assert!(grant.ymm && !grant.movbe);
        let deny = X86Isa::SSE2.with_target_features(&feats(&["arch=haswell", "no-movbe"]));
        assert_eq!(
            deny,
            X86Isa {
                movbe: false,
                ..X86Isa::V3
            }
        );
        // Without `arch=` the TU baseline stays.
        assert_eq!(
            X86Isa::V3.with_target_features(&feats(&["no-movbe"])),
            X86Isa {
                movbe: false,
                ..X86Isa::V3
            }
        );
    }

    /// GCC 16.2 `-march=haswell -mno-X` (`-dM -E`): what each denial leaves.
    #[test]
    fn denials_cascade_like_gcc() {
        let after = |denial: &str| X86Isa::V3.with_target_features(&feats(&[denial]));
        let sse2_only = X86Isa {
            movbe: true,
            cx16: true,
            ..X86Isa::SSE2
        };
        let sse41_only = X86Isa {
            sse41: true,
            ..sse2_only
        };
        for denial in ["no-sse3", "no-ssse3", "no-sse4", "no-sse4.1"] {
            assert_eq!(after(denial), sse2_only, "{denial}");
        }
        for denial in ["no-sse4.2", "no-avx"] {
            assert_eq!(after(denial), sse41_only, "{denial}");
        }
        assert_eq!(
            after("no-avx2"),
            X86Isa {
                ymm: false,
                ..X86Isa::V3
            }
        );
        assert!(!after("no-sse2").simd && !after("no-sse2").sse41);
    }

    #[test]
    fn arch_table_is_sorted_unique_and_normalized() {
        let table = super::super::arch_table::X86_ARCH_TABLE;
        assert!(table.windows(2).all(|w| w[0].name < w[1].name));
        use crate::common::x86_features::X86Feature as F;
        for row in table {
            // GCC's measured sets respect the implications lccc relies on.
            let implied = [
                (F::Sse2, F::Sse),
                (F::Sse3, F::Sse2),
                (F::Ssse3, F::Sse3),
                (F::Sse41, F::Ssse3),
                (F::Sse42, F::Sse41),
                (F::Avx, F::Sse42),
                (F::Avx2, F::Avx),
                (F::Fma, F::Avx),
                (F::F16c, F::Avx),
                (F::Vaes, F::Avx),
                (F::Aes, F::Sse2),
            ];
            for (feature, base) in implied {
                assert!(
                    !row.has(feature) || row.has(base),
                    "{} {feature:?}",
                    row.name
                );
            }
            // Every x86-64 CPU has SSE2, CMOV and FXSR.
            assert!(
                !row.x86_64 || (row.has(F::Sse2) && row.has(F::Cmov) && row.has(F::Fxsr)),
                "{}",
                row.name
            );
            assert!(row.x86_64 || row.isa() == X86Isa::NONE, "{}", row.name);
            assert_eq!(row.isa(), row.isa().normalized(), "{}", row.name);
            assert_eq!(X86Isa::arch_row(row.name), Some(row));
        }
        // GCC 16 dropped AES from Westmere..Broadwell (`-march=haswell`
        // does not define `__AES__`); Skylake has it.
        let has = |cpu: &str, f: F| X86Isa::arch_row(cpu).is_some_and(|row| row.has(f));
        assert!(!has("haswell", F::Aes) && has("haswell", F::Pclmul));
        assert!(has("skylake", F::Aes) && has("alderlake", F::Gfni));
        assert!(has("i686", F::Cmov) && !has("pentium", F::Cmov));
    }

    /// The `x86-64-v4` contract, explicit: lccc emits no AVX-512 code, so
    /// every AVX-512 CPU (v4 and the named parts) grants exactly the v3
    /// permission. A v4 `target_clones` clone is dispatched on the full v4
    /// feature set by its resolver (`target_clones::resolve_clone_spec`) and
    /// compiled to v3 code, which every v4 CPU executes.
    #[test]
    fn avx512_cpus_grant_exactly_v3() {
        let table = super::super::arch_table::X86_ARCH_TABLE;
        let avx512: Vec<_> = table
            .iter()
            .filter(|row| row.has(X86Feature::Avx512F))
            .collect();
        assert!(avx512.iter().any(|row| row.name == "x86-64-v4"));
        assert!(avx512.iter().any(|row| row.name == "znver5"));
        for row in avx512 {
            assert_eq!(row.isa(), X86Isa::V3, "{}", row.name);
        }
    }

    #[test]
    fn meet_is_the_fieldwise_and_and_normalized() {
        let no_movbe = X86Isa {
            movbe: false,
            ..X86Isa::V3
        };
        assert_eq!(X86Isa::V3.meet(no_movbe), no_movbe);
        assert_eq!(X86Isa::V3.meet(X86Isa::SSE2), X86Isa::SSE2);
        assert_eq!(X86Isa::V3.meet(X86Isa::NONE), X86Isa::NONE);
        assert_eq!(X86Isa::SSE2.meet(X86Isa::V3), X86Isa::V3.meet(X86Isa::SSE2));
        // Dropping AVX drops everything that implies it.
        let no_avx = X86Isa {
            avx: false,
            ..X86Isa::V3
        };
        let m = X86Isa::V3.meet(no_avx);
        assert!(!m.avx && !m.ymm && !m.fma && m.sse41 && m.movbe);
    }

    #[test]
    fn normalization_enforces_implication_chain() {
        let bogus = X86Isa {
            simd: false,
            sse41: true,
            avx: true,
            ymm: true,
            fma: true,
            movbe: false,
            cx16: false,
        };
        assert_eq!(bogus.normalized(), X86Isa::NONE);
        let no_vex_fma = X86Isa {
            simd: true,
            sse41: true,
            avx: false,
            ymm: true,
            fma: true,
            movbe: false,
            cx16: false,
        };
        let n = no_vex_fma.normalized();
        assert!(n.simd && n.sse41 && !n.avx && !n.ymm && !n.fma);
    }

    #[test]
    fn local_sse_and_avx_denials_revoke_fma_prerequisites() {
        for denied in ["no-sse3", "no-ssse3", "no-sse4.1", "no-sse4.2", "no-avx"] {
            let features = vec!["fma".to_string(), denied.to_string()];
            let isa = X86Isa::SSE2.with_target_features(&features);
            assert!(
                !isa.avx && !isa.fma,
                "{denied} left VEX FMA enabled: {isa:?}"
            );
        }
        // Options apply in order: a later `fma` grants AVX back (GCC 16.2
        // compiles `_mm_fmadd_ps` under `target("no-avx,fma")` to
        // `vfmadd132ps`, measured).
        let granted_after_denial =
            X86Isa::SSE2.with_target_features(&["no-avx".to_string(), "fma".to_string()]);
        assert!(granted_after_denial.avx && granted_after_denial.fma);
    }

    #[test]
    fn bits_round_trip() {
        for bits in 0u8..128 {
            let isa = X86Isa::from_bits(bits).normalized();
            assert_eq!(X86Isa::from_bits(isa.to_bits()), isa);
        }
        assert_eq!(X86Isa::from_bits(X86Isa::V3.to_bits()), X86Isa::V3);
        assert_eq!(X86Isa::from_bits(X86Isa::SSE2.to_bits()), X86Isa::SSE2);
    }

    #[test]
    fn default_is_the_plain_x86_64_baseline() {
        // The policy lock.  Emitters and peepholes that never go through the
        // driver see SSE2 and nothing newer, so no post-baseline instruction
        // can be emitted by a default build.  A silent change here would
        // re-arm the v3 default and emit BMI/AVX code for a plain `-O2`.
        assert_eq!(X86Isa::default(), X86Isa::SSE2);
        let d = X86Isa::default();
        assert!(d.simd, "SSE2 is the x86-64 baseline and stays on");
        assert!(!d.sse41 && !d.avx && !d.ymm && !d.fma);
        assert!(!d.movbe && !d.cx16);
        // The same lock at compile time: the level constants are `const`, so
        // the compiler checks these while building the test binary.
        const { assert!(!X86Isa::SSE2.movbe && !X86Isa::SSE2.cx16) };
        const { assert!(X86Isa::V3.ymm && X86Isa::V3.cx16) };
    }

    #[test]
    fn cx16_is_independent_of_the_simd_chain() {
        // CMPXCHG16B is integer ISA: `-mno-sse` must not take it away, and
        // granting it must not drag the SIMD world in.  A plain x86-64 TU
        // does not get it; a v2+ profile does.
        let no_simd_but_cx16 = X86Isa {
            cx16: true,
            ..X86Isa::NONE
        };
        let n = no_simd_but_cx16.normalized();
        assert!(n.cx16 && !n.simd && !n.sse41 && !n.avx && !n.ymm && !n.fma);
        // The bit is its own.  The process-global emitter mirror packs the
        // whole permission set into one byte, so a collision with MOVBE or a
        // SIMD bit would silently grant one class when the other is asked
        // for.  Assert the packed forms are disjoint and independently
        // recoverable.
        let cx16_only = X86Isa {
            cx16: true,
            ..X86Isa::NONE
        }
        .to_bits();
        let movbe_only = X86Isa {
            movbe: true,
            ..X86Isa::NONE
        }
        .to_bits();
        assert_ne!(cx16_only, movbe_only);
        assert_eq!(cx16_only & movbe_only, 0);
        let both = X86Isa::from_bits(cx16_only | movbe_only);
        assert!(both.cx16 && both.movbe);
        const { assert!(!X86Isa::SSE2.cx16) };
        const { assert!(X86Isa::V3.cx16) };
    }

    #[test]
    fn movbe_is_independent_of_the_simd_chain() {
        // `-mno-sse` must not take MOVBE away (it is an integer load), and
        // `-mno-movbe` must not take the SIMD world away.  The level constant
        // for a plain x86-64 TU does not grant it.
        let no_simd_but_movbe = X86Isa {
            movbe: true,
            cx16: false,
            ..X86Isa::NONE
        };
        assert!(no_simd_but_movbe.normalized().movbe);
        assert!(!no_simd_but_movbe.normalized().simd);
        // Compile-time facts, not runtime ones: the level constants are
        // `const`, so the compiler asserts them while building the test.
        const { assert!(!X86Isa::SSE2.movbe) };
        const { assert!(X86Isa::V3.movbe) };
    }
}

#[cfg(test)]
mod function_feature_tests {
    use super::{X86Isa, features_permit, function_feature_bits};
    use crate::common::x86_features::{EffectiveFeatures, X86Feature as F};
    use crate::ir::module::normalize_target_features;

    fn cpu(name: &str) -> u64 {
        X86Isa::arch_row(name).expect("GCC CPU").feature_bits()
    }

    fn bit(feature: F) -> u64 {
        1 << feature as u8
    }

    /// The features of `target(target)` in an x86-64 TU with features `tu`,
    /// of which `explicit` came from explicit `-m` options.
    fn features(target: &str, tu: u64, explicit: u64) -> EffectiveFeatures {
        let options = normalize_target_features(target);
        EffectiveFeatures::from_raw(function_feature_bits(tu, explicit, &options, true))
    }

    fn permitted(target: &str, feature: F, tu: u64) -> bool {
        features_permit(features(target, tu, 0), feature)
    }

    /// Each verdict is GCC 16.2's (`gcc-16 -c`, intrinsic accepted or
    /// "target specific option mismatch" / "needs isa option").
    #[test]
    fn verdicts_match_gcc() {
        let (v1, hsw) = (cpu("x86-64"), cpu("haswell"));
        let v1_popcnt_avx2 = features("popcnt,avx2", v1, 0).raw();
        // `arch=` discards the TU's grants (`-mpopcnt`, `-mavx2`) ...
        assert!(!permitted("arch=x86-64", F::Popcnt, v1_popcnt_avx2));
        assert!(!permitted("arch=x86-64", F::Avx2, v1_popcnt_avx2));
        // ... and the options before it, while later ones apply.
        assert!(!permitted("avx2,arch=x86-64", F::Avx2, v1));
        assert!(permitted("arch=x86-64,avx2", F::Avx2, v1));
        assert!(permitted("no-avx2,arch=haswell", F::Avx2, v1));
        assert!(!permitted("arch=haswell,no-avx2", F::Avx2, hsw));
        // GCC 16's Haswell and Westmere have no AES; Skylake does.
        assert!(!permitted("arch=haswell", F::Aes, hsw));
        assert!(!permitted("arch=westmere", F::Aes, hsw));
        assert!(permitted("arch=westmere", F::Pclmul, v1));
        assert!(permitted("arch=skylake", F::Aes, v1));
        assert!(permitted("arch=haswell", F::Bmi2, v1));
        // A name GCC rejects on x86-64 (diagnosed when the attribute is
        // lowered) never widens past the x86-64 baseline.
        for bogus in ["arch=pentium4", "arch=bogus"] {
            assert!(permitted(bogus, F::Sse2, hsw) && !permitted(bogus, F::Sse3, hsw));
        }
        // Denials cascade to what is built on the level ...
        assert!(!permitted("no-sse2", F::Sse3, hsw));
        assert!(permitted("no-avx", F::Pclmul, hsw));
        assert!(!permitted("no-avx", F::Fma, hsw));
        assert!(permitted("no-sse4.2", F::Popcnt, hsw));
        assert!(!permitted("no-sse4", F::Sse41, hsw));
        assert!(permitted("no-sse4", F::Ssse3, hsw));
        assert!(permitted("no-sse4.2", F::Sse41, hsw));
        // ... and AES/PCLMUL/GFNI keep their bit but need SSE2 as well:
        // order decides, since `aes` grants SSE2 back.
        let skl = cpu("skylake");
        assert!(!permitted("aes,no-sse2", F::Aes, v1));
        assert!(permitted("no-sse2,aes", F::Aes, v1));
        assert!(!permitted("pclmul,no-sse2", F::Pclmul, v1));
        assert!(!permitted("gfni,no-sse2", F::Gfni, v1));
        assert!(permitted("sse2,gfni", F::Gfni, v1));
        assert!(!permitted("no-sse2", F::Aes, skl));
        // `general-regs-only` revokes the SSE class, not integer ISA.
        assert!(!permitted("general-regs-only", F::Sse, hsw));
        assert!(!permitted("general-regs-only", F::Aes, skl));
        assert!(!permitted("aes,general-regs-only", F::Aes, v1));
        assert!(permitted("general-regs-only", F::Bmi2, hsw));
        // GCC refuses its negation: nothing changes.
        assert_eq!(features("no-general-regs-only", hsw, 0).raw(), hsw);
        // Names grant themselves; level grants imply lower levels only.
        assert!(permitted("aes", F::Aes, v1));
        assert!(permitted("avx2", F::Sse41, v1));
        assert!(!permitted("avx2", F::Aes, v1));
        assert!(permitted("sse4", F::Sse42, v1));
        assert!(!permitted("avx2,no-sse4.1", F::Avx2, v1));
        // No attribute: the TU decides.
        assert!(permitted("", F::Avx2, hsw));
        assert!(!permitted("", F::Avx2, v1));
    }

    /// GCC 16.2 applies a `target("...")` in order (`gcc-16 -E`, `#ifdef`
    /// after the pragma): the AVX-512 graph is GCC's, measured.
    #[test]
    fn options_apply_in_order_with_gccs_graph() {
        let v1 = cpu("x86-64");
        // AVX512BMM brings AVX512BW, AVX512F and the AVX2 chain ...
        let bmm = features("avx512bmm", v1, 0);
        for f in [F::Avx512Bmm, F::Avx512Bw, F::Avx512F, F::Avx2, F::Sse42] {
            assert!(bmm.contains(f), "{f:?}");
        }
        assert!(!bmm.contains(F::Avx512Vl));
        // ... and revoking it later keeps them.
        let undone = features("avx512bmm,no-avx512bmm", v1, 0);
        assert!(!undone.contains(F::Avx512Bmm) && undone.contains(F::Avx512Bw));
        // `no-avx2` revokes every AVX-512 subset, whatever came first.
        assert!(!permitted("avx512f,no-avx2", F::Avx512F, v1));
        assert!(!permitted("avx512bw,no-avx512f", F::Avx512Bw, v1));
        assert!(permitted("no-avx2,avx512bw", F::Avx2, v1));
        // AVX512VP2INTERSECT brings DQ; the dropped Xeon Phi subsets AVX2.
        assert!(permitted("avx512vp2intersect", F::Avx512Dq, v1));
        // Duplicates and repeats are harmless; order still decides.
        assert!(permitted("no-avx2,avx2,avx2", F::Avx2, v1));
        // `tune=` and unmodelled names change nothing.
        assert_eq!(features("tune=znver5,sha,rtm", v1, 0).raw(), v1);
    }

    /// GCC's override implications and what blocks them (`gcc-16 -E`).
    #[test]
    fn implications_yield_to_explicit_options() {
        let i386 = 0;
        let (v1, hsw) = (cpu("x86-64"), cpu("haswell"));
        let popcnt = bit(F::Popcnt);
        // SSE4.2 brings POPCNT unless anything named POPCNT explicitly: in
        // the attribute (either order) or on the command line ...
        assert!(features("sse4.2", v1, 0).contains(F::Popcnt));
        assert!(!features("sse4.2,no-popcnt", v1, 0).contains(F::Popcnt));
        assert!(!features("no-popcnt,sse4.2", v1, 0).contains(F::Popcnt));
        assert!(!features("sse4.2", v1, popcnt).contains(F::Popcnt));
        // ... where an `arch=` forgets it (`-mno-popcnt` with
        // `target("arch=x86-64,sse4.2")` has POPCNT).
        assert!(features("arch=x86-64,sse4.2", v1, popcnt).contains(F::Popcnt));
        assert!(features("arch=haswell", v1, popcnt).contains(F::Popcnt));
        // SSE brings MMX; `general-regs-only` makes MMX explicit.
        assert!(features("sse", i386, 0).contains(F::Mmx));
        let gro = features("general-regs-only,sse", i386, 0);
        assert!(gro.contains(F::Sse) && !gro.contains(F::Mmx));
        // ABM grants POPCNT and brings LZCNT; `no-abm` revokes only ABM.
        let abm = features("abm", i386, 0);
        assert!(abm.contains(F::Popcnt) && abm.contains(F::Lzcnt));
        let abm_no_lzcnt = features("abm,no-lzcnt", i386, 0);
        assert!(abm_no_lzcnt.contains(F::Popcnt) && !abm_no_lzcnt.contains(F::Lzcnt));
        let no_abm = features("no-abm", hsw, 0);
        assert!(no_abm.contains(F::Popcnt) && no_abm.contains(F::Lzcnt));
    }

    #[test]
    fn evex_dot_products_need_vnni_and_vl() {
        let evex = |target: &str, tu: u64| {
            let set = features(target, tu, 0);
            [F::Avx512Vnni, F::Avx512Vl, F::Avx2]
                .iter()
                .all(|&f| set.contains(f))
        };
        let clx = cpu("cascadelake");
        assert!(evex("", clx));
        assert!(!evex("", cpu("alderlake")));
        assert!(evex("avx512vnni,avx512vl", cpu("x86-64")));
        assert!(!evex("avx512vnni", cpu("x86-64")));
        assert!(!evex("arch=alderlake", clx));
        for denial in [
            "no-avx512vl",
            "no-avx512vnni",
            "no-avx512f",
            "no-avx2",
            "no-sse4.2",
        ] {
            assert!(!evex(denial, clx), "{denial}");
        }
        assert!(!evex("general-regs-only", clx));
        // AVX-VNNI is its own CPUID bit: Alder Lake has it, Cascade Lake not.
        assert!(permitted("arch=alderlake", F::AvxVnni, 0));
        assert!(!permitted("arch=cascadelake", F::AvxVnni, cpu("alderlake")));
    }

    #[test]
    fn i686_accepts_32_bit_cpus() {
        let options = normalize_target_features("arch=pentium4");
        let set = EffectiveFeatures::from_raw(function_feature_bits(0, 0, &options, false));
        assert!(set.contains(F::Sse2) && !set.contains(F::Sse3));
        // An unknown name grants nothing on i686 (no SSE2 floor there).
        let bogus = normalize_target_features("arch=bogus");
        assert_eq!(function_feature_bits(cpu("pentium4"), 0, &bogus, false), 0);
    }

    /// A CPU's ABM-derived POPCNT and LZCNT leave with ABM after its own
    /// `arch=`, unless an option named them (gcc-16 `#pragma GCC target`,
    /// measured); without an `arch=`, `no-abm` revokes ABM alone.
    #[test]
    fn no_abm_after_arch_withholds_the_cpus_abm_derived_bits() {
        use crate::backend::x86::arch_table::ABM;
        let bits = |tu: u64, options: &[&str]| {
            let options: Vec<String> = options.iter().map(|o| (*o).to_string()).collect();
            let set = function_feature_bits(tu, 0, &options, true);
            [
                set & ABM != 0,
                set & (1 << F::Popcnt as u8) != 0,
                set & (1 << F::Lzcnt as u8) != 0,
            ]
        };
        let amdfam10 = X86Isa::arch_row("amdfam10").unwrap().feature_bits();
        assert_eq!(bits(0, &["arch=amdfam10"]), [true, true, true]);
        assert_eq!(bits(0, &["arch=amdfam10", "no-abm"]), [false, false, false]);
        assert_eq!(
            bits(0, &["arch=amdfam10", "popcnt", "no-abm"]),
            [false, true, false]
        );
        assert_eq!(
            bits(0, &["arch=amdfam10", "abm", "no-abm"]),
            [false, true, false]
        );
        assert_eq!(bits(0, &["no-abm", "arch=amdfam10"]), [true, true, true]);
        assert_eq!(bits(0, &["arch=btver1", "no-abm"]), [false, true, true]);
        assert_eq!(bits(amdfam10, &["no-abm"]), [false, true, true]);
    }
}
