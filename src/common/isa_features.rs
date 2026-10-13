//! Effective x86 ISA feature set of the current compilation (AUD-CPU-1).
//!
//! `__builtin_cpu_supports("f")` is a *runtime* question in GCC: the compiler
//! emits a load of libgcc's `__cpu_model` feature bits, so a binary compiled
//! for any target still only takes an AVX2/SHA path on a CPU that has it.
//! LCCC has no runtime CPU model yet, so it must answer statically instead —
//! and the only sound static answer is "1 iff the feature is guaranteed by the
//! code-generation target we actually compiled this translation unit for".
//!
//! The previous implementation folded against a hard-coded Raptor Lake
//! allowlist (`expr_builtins.rs`), independent of `-march`/`-m*`/target.  A
//! `-march=x86-64 -mno-avx2` build still answered `1` for `"avx2"`, so every
//! runtime-dispatched library (zlib-ng, zstd, xz, OpenSSL, FFmpeg) took its
//! AVX2 path unconditionally and raised `#UD` on any CPU without AVX2.
//!
//! Invariants this module maintains:
//!
//!   * The answer is derived from the SAME feature state that gates code
//!     generation (the driver's `enable_*` fields plus an explicit
//!     `-mno-sse`/`-mgeneral-regs-only` denial), never from a CPU name.
//!   * A feature the compiler does not model at all (SHA-NI, ADX, RDSEED,
//!     AVX512-BF16 on a backend without intrinsic coverage, ...) folds to 0.
//!     Dispatchers then take their generic path: slower, never illegal.
//!   * Non-x86 targets fold every x86 feature name to 0 (AArch64/RISC-V have
//!     no such runtime property; a real diagnostic is the follow-up work in
//!     AUD-SILENT-ZERO-1, not a silent `1`).
//!
//! Follow-up (FE-25, tracked in `engineering/DECISIONS.md`): lower the
//! *unresolved* cases to a runtime CPUID test, which is what GCC does and what
//! restores the dispatch performance this conservative fold gives up.
use std::sync::atomic::{AtomicU64, Ordering};

/// Canonical feature names; the index in this table is the bit number.
///
/// Only features the driver can actually guarantee belong here.  Aliases
/// (GCC spellings and LCCC spellings) are resolved by [`canonical_name`].
pub const FEATURE_TABLE: &[&str] = &[
    "cmov",
    "mmx",
    "xsave",
    "osxsave",
    "sse",
    "sse2",
    "fxsr",
    "cx16",
    "sse3",
    "ssse3",
    "sse4.1",
    "sse4.2",
    "popcnt",
    "avx",
    "avx2",
    "f16c",
    "fma",
    "bmi",
    "bmi2",
    "lzcnt",
    "movbe",
    "aes",
    "pclmul",
    "rdrnd",
    "avxvnni",
    "gfni",
    "vaes",
    "vpclmulqdq",
    "avxifma",
    "avxneconvert",
    "avxvnniint8",
    "avxvnniint16",
    "sm3",
    "sm4",
    "cmpccxadd",
    "avx512f",
    "avx512cd",
    "avx512dq",
    "avx512bw",
    "avx512vl",
    "avx512ifma",
    "avx512vbmi",
    "avx512vbmi2",
    "avx512vnni",
    "avx512bitalg",
    "avx512vpopcntdq",
    "avx512bf16",
    "avx512fp16",
    "avx512er",
    "avx512pf",
    "avx512vp2intersect",
    "sha512",
    "amx-tile",
    "amx-int8",
    "amx-bf16",
];

const MAX_FEATURES: usize = 64;

/// Map a GCC spelling of a feature to its canonical table name.
pub fn canonical_name(name: &str) -> Option<&'static str> {
    let lowered = name.to_ascii_lowercase();
    let name = lowered.as_str();
    let alias = match name {
        "sse4_1" => "sse4.1",
        "sse4_2" => "sse4.2",
        "pclmulqdq" => "pclmul",
        "rdrand" => "rdrnd",
        "avx512-vnni" => "avx512vnni",
        "avx512-bf16" => "avx512bf16",
        "avx512-fp16" => "avx512fp16",
        "amx_tile" => "amx-tile",
        "amx_int8" => "amx-int8",
        "amx_bf16" => "amx-bf16",
        other => other,
    };
    FEATURE_TABLE.iter().copied().find(|f| *f == alias)
}

/// A set of ISA features that code generation may assume unconditionally.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct FeatureSet(u64);

impl FeatureSet {
    pub const fn empty() -> Self {
        Self(0)
    }

    fn bit(name: &str) -> Option<u32> {
        let canonical = canonical_name(name)?;
        let index = FEATURE_TABLE
            .iter()
            .position(|f| *f == canonical)
            .expect("canonical_name returns table entries") as u32;
        debug_assert!((index as usize) < MAX_FEATURES);
        Some(index)
    }

    /// Insert a feature by name; returns false for names outside the table.
    pub fn insert(&mut self, name: &str) -> bool {
        match Self::bit(name) {
            Some(bit) => {
                self.0 |= 1u64 << bit;
                true
            }
            None => false,
        }
    }

    pub fn remove(&mut self, name: &str) -> bool {
        match Self::bit(name) {
            Some(bit) => {
                self.0 &= !(1u64 << bit);
                true
            }
            None => false,
        }
    }

    pub fn contains(&self, name: &str) -> bool {
        Self::bit(name).is_some_and(|bit| self.0 & (1u64 << bit) != 0)
    }

    /// Apply `(name, enabled)` pairs; disabled entries clear the bit first so
    /// that a `-mno-*` denial wins regardless of order.
    pub fn apply<'a, I>(&mut self, features: I)
    where
        I: IntoIterator<Item = (&'a str, bool)>,
    {
        for (name, enabled) in features {
            if enabled {
                self.insert(name);
            } else {
                self.remove(name);
            }
        }
    }

    /// The x86-64 (v1) baseline: every x86-64 CPU implements these, so a
    /// dispatcher may select them without a runtime test.
    pub fn insert_x86_64_baseline(&mut self) {
        for name in ["cmov", "mmx", "sse", "sse2", "fxsr", "cx16"] {
            self.insert(name);
        }
    }

    /// AVX implies XSAVE/OSXSAVE on a CPU that can run the code: the xstate
    /// enables required for VEX instructions are a prerequisite for AVX.
    pub fn insert_avx_implied(&mut self) {
        if self.contains("avx") {
            self.insert("xsave");
            self.insert("osxsave");
        }
    }

    pub fn is_empty(&self) -> bool {
        self.0 == 0
    }
}

static EFFECTIVE: AtomicU64 = AtomicU64::new(0);
static PUBLISHED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Publish the feature set of the translation unit about to be compiled.
///
/// Called by the driver immediately before lowering; `__builtin_cpu_supports`
/// folding reads it.  Process-global because lowering runs on a worker thread.
pub fn publish(set: FeatureSet) {
    EFFECTIVE.store(set.0, Ordering::Release);
    PUBLISHED.store(true, Ordering::Release);
}

/// The published feature set, or `None` when nothing was published yet.
pub fn effective() -> Option<FeatureSet> {
    PUBLISHED
        .load(Ordering::Acquire)
        .then(|| FeatureSet(EFFECTIVE.load(Ordering::Acquire)))
}

#[cfg(test)]
pub fn reset_for_test() {
    PUBLISHED.store(false, Ordering::Release);
    EFFECTIVE.store(0, Ordering::Release);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_names_and_aliases() {
        assert_eq!(canonical_name("SSE4_2"), Some("sse4.2"));
        assert_eq!(canonical_name("sse4.2"), Some("sse4.2"));
        assert_eq!(canonical_name("pclmulqdq"), Some("pclmul"));
        assert_eq!(canonical_name("PCLMUL"), Some("pclmul"));
        assert_eq!(canonical_name("sha"), None);
        assert_eq!(canonical_name("xop"), None);
    }

    #[test]
    fn baseline_is_x86_64_v1() {
        let mut set = FeatureSet::empty();
        set.insert_x86_64_baseline();
        assert!(set.contains("sse2") && set.contains("cmov"));
        assert!(!set.contains("sse3") && !set.contains("avx") && !set.contains("popcnt"));
    }

    #[test]
    fn apply_is_last_write_wins_for_denials() {
        let mut set = FeatureSet::empty();
        set.apply([("avx2", true), ("bmi2", true), ("avx2", false)]);
        assert!(set.contains("bmi2"));
        assert!(!set.contains("avx2"));
    }
}
