//! Canonical name table of the x86 features the *driver model* can grant
//! (the `enable_*`/`resolved_*` flag state), used to keep the runtime CPU
//! probe's name space (`crate::common::cpu_probe`) in sync with the compile
//! flag model.
//!
//! History: this module used to implement the *static fold* of
//! `__builtin_cpu_supports` ("1 iff the codegen permission guarantees the
//! feature").  Measured against GCC 16.2, Clang 23.1 and ICX, that model was
//! wrong — no reference compiler folds, ever, under any flag combination
//! (they all answer from a runtime CPUID probe) — so the fold was removed and
//! `cpu_probe` implements the GCC semantics instead (FE-25 is closed).
//! What remains here is the name registry: every feature the compile-flag
//! model can guarantee must have a runtime probe entry, or
//! `__builtin_cpu_supports("gfni")`-style dispatch silently becomes an
//! unknown-name error on a compiler that can clearly emit GFNI code.
//! `cpu_probe::tests::every_static_feature_name_has_a_runtime_probe` pins the
//! sync in both directions.

/// Canonical feature names the driver's flag model can grant.
///
/// Consumed by the `cpu_probe` sync test (hence `cfg(test)`: nothing in a
/// production build reads this table any more).
#[cfg(test)]
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
