//! Effective x86 feature set for `__builtin_cpu_supports` — AUD-CPU-1.
//!
//! # The bug this replaces
//!
//! `__builtin_cpu_supports("feat")` folded against a hard-coded "Raptor Lake"
//! allowlist in `ir/lowering/expr_builtins.rs`, ignoring `-march`, the `-m*`
//! flags and the target architecture. Measured before the fix with the audit's
//! own probe (`avx2 + 10*bmi2 + 100*sha`):
//!
//! | invocation | folded | problem |
//! |---|---|---|
//! | `-march=x86-64` | `111` | claims AVX2/BMI2/SHA on a v1 target |
//! | `-march=x86-64 -mno-avx2` | `111` | ignores an explicit denial |
//! | `-march=x86-64-v3` / `raptorlake` | `111` | SHA is not part of v3 |
//! | `lccc-arm` (aarch64) | `111` | x86 feature names mean nothing there |
//!
//! Runtime-dispatch code (zlib-ng, zstd, xz, OpenSSL, FFmpeg) therefore always
//! took its widest path, and that path runs on hardware the user never named —
//! a guaranteed `SIGILL`, and a slow, wrong answer in the literal sense.
//!
//! # The model
//!
//! The folded value answers one question: **does the effective code-generation
//! permission of this invocation guarantee the feature?**
//!
//! * `1` — the backend may emit that instruction class for *any* input
//!   (it is inside the resolved ISA ceiling), so a binary containing it could
//!   not run on hardware without the feature. Claiming `1` is exactly as safe
//!   as the rest of the emitted code.
//! * `0` — an explicit denial, a `-march=` ceiling below the feature, a target
//!   without it, or a feature this compiler does not track at all. Dispatchers
//!   take the generic path: always correct.
//!
//! The two directions are handled by two producers, and neither is a second
//! table: the driver's [`crate::driver::Driver::effective_x86_features`]
//! evaluates the *same* predicates the backend uses — `x86_isa()` for the
//! VEX/xmm classes, `resolved_bmi1`/`resolved_bmi2`/`resolved_lzcnt`/
//! `resolved_movbe`/`resolved_popcnt` for the integer features, and the
//! `__<FEAT>__` macro predicates for the rungs that have no codegen bit of
//! their own. What the set is used for, and what is deliberately pinned:
//!
//! * the **intrinsic** permission gate (AUD-CPU-3): an x86 intrinsic whose
//!   extension this set does not contain is diagnosed rather than emitted
//!   (`tests/regression/check_intrinsic_feature_gate.sh`).  It is *not* the
//!   `__builtin_cpu_supports` answer — that builtin probes CPUID at run time,
//!   matching GCC (AUD-CPU-1, `common::cpu_probe`; see
//!   `tests/regression/check_cpu_supports_runtime.sh`).
//!
//! * no `-march=` → the project code-generation baseline, **x86-64-v3**
//!   (`backend::x86::isa` documents it: every benchmark number in
//!   `engineering/` depends on it), so AVX2/BMI2/FMA/F16C fold to `1`;
//! * `-march=x86-64` / `-march=x86-64-v2` → v1 / v2 exactly;
//! * `-mno-<feat>` is sticky against both the baseline and a later `-march=`,
//!   and a baseline grant never resurrects an explicitly denied rung
//!   (`-mno-sse4.1 -msse3` used to report SSE4.1);
//! * non-x86 targets (aarch64, riscv64, ...) get a diagnostic instead of a
//!   silent `0`, and so does a non-literal feature-name argument.
//!
//! # Deliberate deviations from GCC
//!
//! GCC lowers this builtin to a *runtime* CPUID probe (`__cpu_model`). lccc
//! folds against the compile-time guarantee, which is conservative wherever
//! the host has more than the target promises: `-march=x86-64-v3` reports
//! `sha = 0` even when built on a Raptor Lake host that has SHA-NI. Two more
//! consequences are worth stating because they are visible:
//!
//! * Features with no codegen permission bit fold to `0` even for a Raptor
//!   Lake profile: `sha`, `adx`, `rdseed`, `fsgsbase`, `xsave*`, `gfni`,
//!   `avxvnni`, unmodeled AVX-512 subsets, `amx*`, `xop`, `sse4a`, `tbm`, ...
//!   A `1` for an instruction the backend cannot promise is the bug being fixed here;
//!   `-msha` is rejected outright for the same reason.
//! * `vaes`/`vpclmulqdq` are tracked, but as the *256-bit* permission they
//!   actually are: each requires the 256-bit vector class in addition to its
//!   own `-m` flag, because the 128-bit encodings lccc emits are AES-NI and
//!   PCLMUL and do not need VAES.  GCC's verdicts agree (`-mvaes` alone
//!   rejects `_mm256_aesenc_epi128`; `-mavx2 -mvaes` and `-mavx512f -mvaes`
//!   accept it), and `-mvaes` without AVX2 must not fold to `1`: the fold
//!   promises what the backend may emit, and without the 256-bit class it may
//!   not emit this one.
//! * Because SSE4.2 has no codegen bit of its own, a denial of the SSE4.2-era
//!   surface (`-mno-sse4.2`, `-mno-avx`) suppresses the *baseline* grant of
//!   `sse4.2` even though legacy SSE4.2 encodings stay legal and the
//!   `__SSE4_2__` macro stays defined. Under-reporting only costs a dispatch
//!   to the generic path.
//!
//! The runtime path (libgcc-compatible `__cpu_model` / inline CPUID+XGETBV) is
//! tracked as **AUD-CPU-2** in the follow-up document.
//!
//! # Known adjacent hole (not this fix)
//!
//! The *intrinsic* surface is not ISA-gated in lccc: `_mm_crc32_u8` under
//! `-march=x86-64` still emits `crc32b` (measured), i.e. an SSE4.2 instruction
//! in a v1 object. The fold answers truthfully about the permission, but the
//! permission itself has a hole. That is AUD-CPU-3 (intrinsic-class ISA
//! gating), recorded with this reproducer in the follow-up document.

use std::sync::atomic::{AtomicU64, Ordering};

/// A feature name accepted by `__builtin_cpu_supports`.
///
/// The set is exactly what the driver can resolve from flags; see the module
/// documentation for how the untracked names behave.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum X86Feature {
    Cmov,
    Fxsr,
    Mmx,
    Sse,
    Sse2,
    Sse3,
    Ssse3,
    Sse41,
    Sse42,
    Popcnt,
    Cx16,
    Avx,
    Avx2,
    Fma,
    F16c,
    Bmi,
    Bmi2,
    Lzcnt,
    Movbe,
    Aes,
    Pclmul,
    Rdrnd,
    Vaes,
    Vpclmulqdq,
    /// AVX-VNNI: the VEX-encoded dot-product family, `vpdpbusd`/`vpdpbusds`/
    /// `vpdpwssd`/`vpdpwssds`.  Its own CPUID bit (leaf 7, EBX bit 4) and its
    /// own `-mavxvnni` flag; VEX-encoded, so it needs a vector class too.
    AvxVnni,
    /// AVX-VNNI-INT8: `vpdpbssd(s)`, `vpdpbsud(s)`, `vpdpbuud(s)` -- the byte
    /// forms where either input may be signed.  A separate CPUID bit and flag
    /// (`-mavxvnniint8`), not a synonym of AVX-VNNI.
    AvxVnniInt8,
    /// AVX-VNNI-INT16: `vpdpwusd(s)`, `vpdpwuud(s)` -- the unsigned-word
    /// forms.  A third, separate bit and flag (`-mavxvnniint16`).
    AvxVnniInt16,
    /// GFNI: the Galois-field byte instructions (`gf2p8mulb`,
    /// `gf2p8affineqb`, `gf2p8affineinvqb`).  Legacy-encoded for xmm, so
    /// `-mno-avx` does not remove it, and a separate CPUID bit and flag
    /// (`-mgfni`) from the VNNI families.
    Gfni,
    /// AVX-512 Foundation (EVEX/ZMM base instruction set).
    Avx512F,
    /// AVX-512 Byte and Word instructions.
    Avx512Bw,
    /// AVX-512 Doubleword and Quadword instructions.
    Avx512Dq,
    /// AVX-512 Vector Length (EVEX.128/256 forms).
    Avx512Vl,
    /// AVX-512 Vector Neural Network Instructions.
    Avx512Vnni,
    /// AVX-512 Bit Algorithms (byte/word population count and bit tests).
    Avx512BitAlg,
    /// AVX-512 Vector Population Count (dword/qword forms).
    Avx512Vpopcntdq,
    /// AVX-512 Conflict Detection.
    Avx512Cd,
    /// AVX-512 Integer Fused Multiply Add.
    Avx512Ifma,
    /// AVX-512 Vector Byte Manipulation Instructions.
    Avx512Vbmi,
    /// AVX-512 Vector Byte Manipulation Instructions 2.
    Avx512Vbmi2,
    /// AVX-512 Bfloat16 instructions.
    Avx512Bf16,
    /// AVX-512 FP16 instructions.
    Avx512Fp16,
    /// AVX-512 Bit Matrix Multiply (GCC 16.2 feature).
    Avx512Bmm,
    /// AVX-512 Vector Pair Intersection.
    Avx512Vp2intersect,
    /// AVX-512 Exponential and Reciprocal Approximation (legacy).
    Avx512Er,
    /// AVX-512 Prefetch Instructions (legacy).
    Avx512Pf,
}

impl X86Feature {
    /// Every tracked feature, for table-driven tests.
    pub const ALL: [X86Feature; 45] = [
        X86Feature::Cmov,
        X86Feature::Fxsr,
        X86Feature::Mmx,
        X86Feature::Sse,
        X86Feature::Sse2,
        X86Feature::Sse3,
        X86Feature::Ssse3,
        X86Feature::Sse41,
        X86Feature::Sse42,
        X86Feature::Popcnt,
        X86Feature::Cx16,
        X86Feature::Avx,
        X86Feature::Avx2,
        X86Feature::Fma,
        X86Feature::F16c,
        X86Feature::Bmi,
        X86Feature::Bmi2,
        X86Feature::Lzcnt,
        X86Feature::Movbe,
        X86Feature::Aes,
        X86Feature::Pclmul,
        X86Feature::Rdrnd,
        X86Feature::Vaes,
        X86Feature::Vpclmulqdq,
        X86Feature::AvxVnni,
        X86Feature::AvxVnniInt8,
        X86Feature::AvxVnniInt16,
        X86Feature::Gfni,
        X86Feature::Avx512F,
        X86Feature::Avx512Bw,
        X86Feature::Avx512Dq,
        X86Feature::Avx512Vl,
        X86Feature::Avx512Vnni,
        X86Feature::Avx512BitAlg,
        X86Feature::Avx512Vpopcntdq,
        X86Feature::Avx512Cd,
        X86Feature::Avx512Ifma,
        X86Feature::Avx512Vbmi,
        X86Feature::Avx512Vbmi2,
        X86Feature::Avx512Bf16,
        X86Feature::Avx512Fp16,
        X86Feature::Avx512Bmm,
        X86Feature::Avx512Vp2intersect,
        X86Feature::Avx512Er,
        X86Feature::Avx512Pf,
    ];

    /// Canonical GCC spelling of the feature (the one `-m` flags use).
    pub fn name(self) -> &'static str {
        match self {
            X86Feature::Cmov => "cmov",
            X86Feature::Fxsr => "fxsr",
            X86Feature::Mmx => "mmx",
            X86Feature::Sse => "sse",
            X86Feature::Sse2 => "sse2",
            X86Feature::Sse3 => "sse3",
            X86Feature::Ssse3 => "ssse3",
            X86Feature::Sse41 => "sse4.1",
            X86Feature::Sse42 => "sse4.2",
            X86Feature::Popcnt => "popcnt",
            X86Feature::Cx16 => "cx16",
            X86Feature::Avx => "avx",
            X86Feature::Avx2 => "avx2",
            X86Feature::Fma => "fma",
            X86Feature::F16c => "f16c",
            X86Feature::Bmi => "bmi",
            X86Feature::Bmi2 => "bmi2",
            X86Feature::Lzcnt => "lzcnt",
            X86Feature::Movbe => "movbe",
            X86Feature::Aes => "aes",
            X86Feature::Pclmul => "pclmul",
            X86Feature::Rdrnd => "rdrnd",
            X86Feature::Vaes => "vaes",
            X86Feature::Vpclmulqdq => "vpclmulqdq",
            X86Feature::AvxVnni => "avxvnni",
            X86Feature::AvxVnniInt8 => "avxvnniint8",
            X86Feature::AvxVnniInt16 => "avxvnniint16",
            X86Feature::Gfni => "gfni",
            X86Feature::Avx512F => "avx512f",
            X86Feature::Avx512Bw => "avx512bw",
            X86Feature::Avx512Dq => "avx512dq",
            X86Feature::Avx512Vl => "avx512vl",
            X86Feature::Avx512Vnni => "avx512vnni",
            X86Feature::Avx512BitAlg => "avx512bitalg",
            X86Feature::Avx512Vpopcntdq => "avx512vpopcntdq",
            X86Feature::Avx512Cd => "avx512cd",
            X86Feature::Avx512Ifma => "avx512ifma",
            X86Feature::Avx512Vbmi => "avx512vbmi",
            X86Feature::Avx512Vbmi2 => "avx512vbmi2",
            X86Feature::Avx512Bf16 => "avx512bf16",
            X86Feature::Avx512Fp16 => "avx512fp16",
            X86Feature::Avx512Bmm => "avx512bmm",
            X86Feature::Avx512Vp2intersect => "avx512vp2intersect",
            X86Feature::Avx512Er => "avx512er",
            X86Feature::Avx512Pf => "avx512pf",
        }
    }

    /// Parse the string literal a translation unit passes to the builtin.
    ///
    /// GCC accepts the `__builtin_cpu_supports` spellings below; the aliases
    /// are kept because the previous hard-coded table accepted them and real
    /// dispatch code uses both (`pclmul` is GCC's, `pclmulqdq` is Clang's and
    /// Intel's).
    pub fn parse(name: &str) -> Option<Self> {
        let n = name.to_ascii_lowercase();
        Some(match n.as_str() {
            "cmov" => X86Feature::Cmov,
            "fxsr" => X86Feature::Fxsr,
            "mmx" => X86Feature::Mmx,
            "sse" => X86Feature::Sse,
            "sse2" => X86Feature::Sse2,
            "sse3" | "pni" => X86Feature::Sse3,
            "ssse3" => X86Feature::Ssse3,
            "sse4.1" | "sse4_1" => X86Feature::Sse41,
            "sse4.2" | "sse4_2" => X86Feature::Sse42,
            "popcnt" => X86Feature::Popcnt,
            "cx16" => X86Feature::Cx16,
            "avx" => X86Feature::Avx,
            "avx2" => X86Feature::Avx2,
            "fma" | "fma3" => X86Feature::Fma,
            "f16c" => X86Feature::F16c,
            "bmi" => X86Feature::Bmi,
            "bmi2" => X86Feature::Bmi2,
            "lzcnt" => X86Feature::Lzcnt,
            "movbe" => X86Feature::Movbe,
            "aes" => X86Feature::Aes,
            "pclmul" | "pclmulqdq" => X86Feature::Pclmul,
            "rdrnd" | "rdrand" => X86Feature::Rdrnd,
            "vaes" => X86Feature::Vaes,
            "vpclmulqdq" | "vpclmul" => X86Feature::Vpclmulqdq,
            "avxvnni" => X86Feature::AvxVnni,
            "avxvnniint8" => X86Feature::AvxVnniInt8,
            "avxvnniint16" => X86Feature::AvxVnniInt16,
            "gfni" => X86Feature::Gfni,
            "avx512f" => X86Feature::Avx512F,
            "avx512bw" => X86Feature::Avx512Bw,
            "avx512dq" => X86Feature::Avx512Dq,
            "avx512vl" => X86Feature::Avx512Vl,
            "avx512vnni" => X86Feature::Avx512Vnni,
            "avx512bitalg" => X86Feature::Avx512BitAlg,
            "avx512vpopcntdq" => X86Feature::Avx512Vpopcntdq,
            "avx512cd" => X86Feature::Avx512Cd,
            "avx512ifma" => X86Feature::Avx512Ifma,
            "avx512vbmi" => X86Feature::Avx512Vbmi,
            "avx512vbmi2" => X86Feature::Avx512Vbmi2,
            "avx512bf16" => X86Feature::Avx512Bf16,
            "avx512fp16" => X86Feature::Avx512Fp16,
            "avx512bmm" => X86Feature::Avx512Bmm,
            "avx512vp2intersect" => X86Feature::Avx512Vp2intersect,
            "avx512er" => X86Feature::Avx512Er,
            "avx512pf" => X86Feature::Avx512Pf,
            _ => return None,
        })
    }

    #[inline]
    fn bit(self) -> u64 {
        1u64 << (self as u8)
    }
}

/// A set of [`X86Feature`]s (bitmask; all tracked features fit in a `u64`).
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct EffectiveFeatures(u64);

impl EffectiveFeatures {
    pub const EMPTY: EffectiveFeatures = EffectiveFeatures(0);

    /// Add a feature to the set.
    pub fn insert(&mut self, feature: X86Feature) {
        self.0 |= feature.bit();
    }

    /// Builder form of [`Self::insert`].
    #[must_use]
    pub fn with(mut self, feature: X86Feature) -> Self {
        self.insert(feature);
        self
    }

    /// Whether the effective permission guarantees `feature`.
    #[inline]
    pub fn contains(self, feature: X86Feature) -> bool {
        self.0 & feature.bit() != 0
    }

    /// Add `feature` when `granted` (keeps the construction sites table-like).
    pub fn insert_if(&mut self, granted: bool, feature: X86Feature) {
        if granted {
            self.insert(feature);
        }
    }

    #[inline]
    pub fn raw(self) -> u64 {
        self.0
    }

    #[inline]
    pub fn from_raw(raw: u64) -> Self {
        EffectiveFeatures(raw)
    }
}

/// Process-global mirror of the *current* invocation's effective feature set.
///
/// Set once per driver run (`Driver::run`), read by IR lowering, which has no
/// handle on the driver (it runs on a worker thread and takes `&self`). This
/// mirrors the existing process-global pattern for target properties
/// (`common::types::set_target_*`, `backend::x86::isa::set_current`).
static EFFECTIVE: AtomicU64 = AtomicU64::new(0);

/// Publish the effective feature set for this compilation.
pub fn set_effective(features: EffectiveFeatures) {
    EFFECTIVE.store(features.raw(), Ordering::Relaxed);
}

/// The effective feature set published by the driver (empty until it runs,
/// i.e. an unknown target guarantees nothing).
#[inline]
pub fn effective() -> EffectiveFeatures {
    EffectiveFeatures::from_raw(EFFECTIVE.load(Ordering::Relaxed))
}

/// The features the command line set or cleared explicitly (GCC's
/// `ix86_isa_flags_explicit`): GCC's option-override implications
/// (`-msse4.2` brings POPCNT) leave them alone in every `target("...")`
/// function without an `arch=`. See `backend::x86::isa::function_feature_bits`.
static EXPLICIT: AtomicU64 = AtomicU64::new(0);

/// Publish the explicit-feature mask for this compilation.
pub fn set_explicit(mask: u64) {
    EXPLICIT.store(mask, Ordering::Relaxed);
}

/// The explicit-feature mask published by the driver (empty until it runs).
#[inline]
pub fn explicit() -> u64 {
    EXPLICIT.load(Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every tracked feature must round-trip through `name()`/`parse()`, so a
    /// typo in either table is a test failure rather than a silent `0`.
    #[test]
    fn every_feature_round_trips_through_its_canonical_name() {
        for feature in X86Feature::ALL {
            assert_eq!(
                X86Feature::parse(feature.name()),
                Some(feature),
                "{} does not parse back to itself",
                feature.name()
            );
        }
    }

    /// The aliases the previous hard-coded table accepted (and that real
    /// dispatch code uses) must keep working.
    #[test]
    fn gcc_and_clang_spellings_are_aliases() {
        assert_eq!(X86Feature::parse("pni"), Some(X86Feature::Sse3));
        assert_eq!(X86Feature::parse("sse4_1"), Some(X86Feature::Sse41));
        assert_eq!(X86Feature::parse("sse4_2"), Some(X86Feature::Sse42));
        assert_eq!(X86Feature::parse("fma3"), Some(X86Feature::Fma));
        assert_eq!(X86Feature::parse("pclmulqdq"), Some(X86Feature::Pclmul));
        assert_eq!(X86Feature::parse("rdrand"), Some(X86Feature::Rdrnd));
        // Clang/ICC spelling of the 256-bit carry-less multiply.
        assert_eq!(X86Feature::parse("vpclmul"), Some(X86Feature::Vpclmulqdq));
        // Case-insensitive, like GCC's own comparison of the literal.
        assert_eq!(X86Feature::parse("AVX2"), Some(X86Feature::Avx2));
    }

    /// Features the compiler has no codegen permission bit for must never be
    /// reportable: a wrong `1` is the SIGILL direction.
    #[test]
    fn untracked_features_are_unknown() {
        for name in [
            "sha",
            "sha_ni",
            "adx",
            "rdseed",
            "fsgsbase",
            "xsave",
            "xsaveopt",
            "avx512gfni",
            "amx_tile",
            "xop",
            "sse4a",
            "tbm",
            "not_a_feature",
            "",
        ] {
            assert_eq!(X86Feature::parse(name), None, "{name} must not be tracked");
        }
    }

    /// AVX-512 features are separate bits, so granting one subset cannot
    /// satisfy the gate for a different subset or vector-length encoding.
    #[test]
    fn avx512_subset_features_are_distinct() {
        for (feature, name) in [
            (X86Feature::Avx512F, "avx512f"),
            (X86Feature::Avx512Bw, "avx512bw"),
            (X86Feature::Avx512Dq, "avx512dq"),
            (X86Feature::Avx512Vl, "avx512vl"),
            (X86Feature::Avx512Vnni, "avx512vnni"),
            (X86Feature::Avx512BitAlg, "avx512bitalg"),
            (X86Feature::Avx512Vpopcntdq, "avx512vpopcntdq"),
            (X86Feature::Avx512Cd, "avx512cd"),
            (X86Feature::Avx512Ifma, "avx512ifma"),
            (X86Feature::Avx512Vbmi, "avx512vbmi"),
            (X86Feature::Avx512Vbmi2, "avx512vbmi2"),
            (X86Feature::Avx512Bf16, "avx512bf16"),
            (X86Feature::Avx512Fp16, "avx512fp16"),
            (X86Feature::Avx512Bmm, "avx512bmm"),
            (X86Feature::Avx512Vp2intersect, "avx512vp2intersect"),
            (X86Feature::Avx512Er, "avx512er"),
            (X86Feature::Avx512Pf, "avx512pf"),
        ] {
            assert_eq!(X86Feature::parse(name), Some(feature));
            assert_eq!(feature.name(), name);
            assert!(X86Feature::ALL.contains(&feature));
            let set = EffectiveFeatures::EMPTY.with(feature);
            assert!(set.contains(feature));
            for other in [
                X86Feature::Avx512F,
                X86Feature::Avx512Bw,
                X86Feature::Avx512Dq,
                X86Feature::Avx512Vl,
                X86Feature::Avx512Vnni,
                X86Feature::Avx512BitAlg,
                X86Feature::Avx512Vpopcntdq,
                X86Feature::Avx512Cd,
                X86Feature::Avx512Ifma,
                X86Feature::Avx512Vbmi,
                X86Feature::Avx512Vbmi2,
                X86Feature::Avx512Bf16,
                X86Feature::Avx512Fp16,
                X86Feature::Avx512Bmm,
                X86Feature::Avx512Vp2intersect,
                X86Feature::Avx512Er,
                X86Feature::Avx512Pf,
            ] {
                if other != feature {
                    assert!(!set.contains(other), "{name} answered for {:?}", other);
                }
            }
        }
    }

    /// GFNI is tracked: it is a legacy-encoded (non-VEX) extension with its
    /// own flag, so the fold must not tie it to the vector class the way the
    /// 256-bit extensions are tied.
    #[test]
    fn gfni_is_tracked_and_separable_from_the_crypto_bits() {
        assert_eq!(X86Feature::parse("gfni"), Some(X86Feature::Gfni));
        assert_eq!(X86Feature::Gfni.name(), "gfni");
        assert!(X86Feature::ALL.contains(&X86Feature::Gfni));
        let mut set = EffectiveFeatures::EMPTY.with(X86Feature::Aes);
        assert!(!set.contains(X86Feature::Gfni));
        set.insert(X86Feature::Gfni);
        assert!(set.contains(X86Feature::Gfni));
        assert!(!set.contains(X86Feature::Pclmul));
    }

    /// The three AVX-VNNI groups are tracked separately and must stay
    /// separate: one bit each, round-tripping through the canonical names a
    /// `__builtin_cpu_supports` call uses, and never answering for a sibling.
    #[test]
    fn the_three_vnni_groups_are_tracked_and_distinct() {
        for (feature, name) in [
            (X86Feature::AvxVnni, "avxvnni"),
            (X86Feature::AvxVnniInt8, "avxvnniint8"),
            (X86Feature::AvxVnniInt16, "avxvnniint16"),
        ] {
            assert_eq!(X86Feature::parse(name), Some(feature));
            assert_eq!(feature.name(), name);
            assert!(X86Feature::ALL.contains(&feature));
            let mut set = EffectiveFeatures::EMPTY;
            set.insert(feature);
            assert!(set.contains(feature));
            for other in [
                X86Feature::AvxVnni,
                X86Feature::AvxVnniInt8,
                X86Feature::AvxVnniInt16,
            ] {
                if other != feature {
                    assert!(!set.contains(other), "{name} answered for {:?}", other);
                }
            }
        }
    }

    /// VAES/VPCLMULQDQ are the 256-bit crypto extensions: tracked, but the
    /// *permission* is a conjunction the driver folds (256-bit vector class
    /// AND the extension's own flag).  The names must resolve so a dispatcher
    /// that asks about them gets a truthful answer instead of `unknown`.
    #[test]
    fn the_256bit_crypto_extensions_are_tracked() {
        assert_eq!(X86Feature::parse("vaes"), Some(X86Feature::Vaes));
        assert_eq!(
            X86Feature::parse("vpclmulqdq"),
            Some(X86Feature::Vpclmulqdq)
        );
        assert_eq!(X86Feature::Vaes.name(), "vaes");
        assert_eq!(X86Feature::Vpclmulqdq.name(), "vpclmulqdq");
        assert!(X86Feature::ALL.contains(&X86Feature::Vaes));
        assert!(X86Feature::ALL.contains(&X86Feature::Vpclmulqdq));
        // The bitmask must keep the two apart, and apart from the 128-bit bits
        // that share their mnemonics: `aes`/`pclmul` must never answer for
        // them (GCC rejects `_mm256_aesenc_epi128` under `-mpclmul`, measured).
        let mut set = EffectiveFeatures::EMPTY.with(X86Feature::Aes);
        assert!(!set.contains(X86Feature::Vaes));
        assert!(!set.contains(X86Feature::Vpclmulqdq));
        set.insert(X86Feature::Vaes);
        set.insert(X86Feature::Vpclmulqdq);
        assert!(set.contains(X86Feature::Vaes));
        assert!(set.contains(X86Feature::Vpclmulqdq));
        assert!(set.contains(X86Feature::Aes));
        // and `pclmul`/`vpclmulqdq` are not the same feature either
        let clmul_only = EffectiveFeatures::EMPTY.with(X86Feature::Pclmul);
        assert!(!clmul_only.contains(X86Feature::Vpclmulqdq));
    }

    #[test]
    fn bitmask_insert_contains_and_raw_round_trip() {
        let mut set = EffectiveFeatures::EMPTY;
        assert!(!set.contains(X86Feature::Avx2));
        set.insert(X86Feature::Avx2);
        set.insert_if(true, X86Feature::Bmi2);
        set.insert_if(false, X86Feature::Sse);
        assert!(set.contains(X86Feature::Avx2));
        assert!(set.contains(X86Feature::Bmi2));
        assert!(!set.contains(X86Feature::Sse));
        assert_eq!(EffectiveFeatures::from_raw(set.raw()), set);
        assert_eq!(
            set,
            EffectiveFeatures::EMPTY
                .with(X86Feature::Avx2)
                .with(X86Feature::Bmi2)
        );
    }

    /// The global is published by the driver before lowering reads it; a
    /// default `0` means an unknown target promises nothing.
    ///
    /// Flipping a process-global from a `#[test]` is only sound when the
    /// window is mutually exclusive with every concurrent reader and restores
    /// on panic — hence the shared lock plus the `Drop` guard (the same
    /// discipline `test_support::backend_flag_lock` documents).
    #[test]
    fn published_set_is_what_lowering_reads() {
        struct Restore(EffectiveFeatures);
        impl Drop for Restore {
            fn drop(&mut self) {
                set_effective(self.0);
            }
        }

        let _lock = crate::test_support::x86_feature_lock();
        let _restore = Restore(effective());

        set_effective(EffectiveFeatures::EMPTY.with(X86Feature::Sse42));
        assert!(effective().contains(X86Feature::Sse42));
        assert!(!effective().contains(X86Feature::Avx2));
    }
}
