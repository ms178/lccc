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
//!   x86-64-v3 (SSE4.2 + AVX2 + FMA3 + BMI2 …).  Every measured benchmark
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

use std::sync::atomic::{AtomicU8, Ordering};

/// Legal x86-64 encodings for the current translation unit.
///
/// The fields are monotone: `ymm ⇒ avx ⇒ sse41 ⇒ simd` and `fma ⇒ avx`.
/// [`X86Isa::normalized`] enforces that so a caller can never assert FMA3
/// while denying VEX.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct X86Isa {
    /// The xmm register file may be touched at all (`-mno-sse` /
    /// `-mgeneral-regs-only` clear it).  When false no vector or scalar-SSE
    /// instruction of any encoding is legal; FP is not expected in such TUs.
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
}

impl X86Isa {
    /// Nothing legal — i686, AArch64, RISC-V and `-mgeneral-regs-only`.
    pub const NONE: X86Isa = X86Isa {
        simd: false,
        sse41: false,
        avx: false,
        ymm: false,
        fma: false,
    };

    /// Plain x86-64 baseline: SSE2 only.
    pub const SSE2: X86Isa = X86Isa {
        simd: true,
        sse41: false,
        avx: false,
        ymm: false,
        fma: false,
    };

    /// x86-64-v3: the project's code-generation baseline.
    pub const V3: X86Isa = X86Isa {
        simd: true,
        sse41: true,
        avx: true,
        ymm: true,
        fma: true,
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
        }
    }

    const BIT_SIMD: u8 = 1 << 0;
    const BIT_SSE41: u8 = 1 << 1;
    const BIT_AVX: u8 = 1 << 2;
    const BIT_YMM: u8 = 1 << 3;
    const BIT_FMA: u8 = 1 << 4;

    const fn to_bits(self) -> u8 {
        let n = self.normalized();
        (if n.simd { Self::BIT_SIMD } else { 0 })
            | (if n.sse41 { Self::BIT_SSE41 } else { 0 })
            | (if n.avx { Self::BIT_AVX } else { 0 })
            | (if n.ymm { Self::BIT_YMM } else { 0 })
            | (if n.fma { Self::BIT_FMA } else { 0 })
    }

    const fn from_bits(b: u8) -> X86Isa {
        X86Isa {
            simd: b & Self::BIT_SIMD != 0,
            sse41: b & Self::BIT_SSE41 != 0,
            avx: b & Self::BIT_AVX != 0,
            ymm: b & Self::BIT_YMM != 0,
            fma: b & Self::BIT_FMA != 0,
        }
    }
}

impl Default for X86Isa {
    /// The project baseline.  Unit tests and isolated emitters that never go
    /// through the driver see v3, i.e. the historical unconditional-VEX
    /// behaviour, so their expectations are unchanged.
    fn default() -> Self {
        X86Isa::V3
    }
}

/// Process-global mirror of the current TU's permission for emitters that
/// have no access to `X86Codegen` (MachInst emission, peephole passes).
static CURRENT: AtomicU8 = AtomicU8::new(X86Isa::V3.to_bits());

/// Publish `isa` for the code paths without `&self`.  Called by
/// `X86Codegen::apply_options` once per translation unit.
pub fn set_current(isa: X86Isa) {
    CURRENT.store(isa.to_bits(), Ordering::Relaxed);
}

/// The permission published by the last [`set_current`] (v3 until then).
#[inline]
pub fn current() -> X86Isa {
    X86Isa::from_bits(CURRENT.load(Ordering::Relaxed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalization_enforces_implication_chain() {
        let bogus = X86Isa {
            simd: false,
            sse41: true,
            avx: true,
            ymm: true,
            fma: true,
        };
        assert_eq!(bogus.normalized(), X86Isa::NONE);
        let no_vex_fma = X86Isa {
            simd: true,
            sse41: true,
            avx: false,
            ymm: true,
            fma: true,
        };
        let n = no_vex_fma.normalized();
        assert!(n.simd && n.sse41 && !n.avx && !n.ymm && !n.fma);
    }

    #[test]
    fn bits_round_trip() {
        for bits in 0u8..32 {
            let isa = X86Isa::from_bits(bits).normalized();
            assert_eq!(X86Isa::from_bits(isa.to_bits()), isa);
        }
        assert_eq!(X86Isa::from_bits(X86Isa::V3.to_bits()), X86Isa::V3);
        assert_eq!(X86Isa::from_bits(X86Isa::SSE2.to_bits()), X86Isa::SSE2);
    }
}
