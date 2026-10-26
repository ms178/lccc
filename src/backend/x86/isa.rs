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

    /// Overlay normalized per-function `__attribute__((target("...")))` features
    /// onto this translation-unit baseline ISA.
    ///
    /// Note that `features` is sorted lexicographically (`arch=...`, `avx*`,
    /// `cx16`, `fma`, `movbe`, `no-*`, `sse*`), so positive grants are applied
    /// in a first pass and explicit `no-*` denials in a second pass before
    /// [`Self::normalized`] enforces the implication chain.
    #[must_use]
    pub fn with_target_features(mut self, features: &[String]) -> X86Isa {
        if features.is_empty() {
            return self;
        }
        for feat in features {
            match feat.as_str() {
                "sse2" | "sse3" | "ssse3" => {
                    self.simd = true;
                }
                "sse4" | "sse4.1" | "sse4.2" => {
                    self.simd = true;
                    self.sse41 = true;
                }
                "avx" => {
                    self.simd = true;
                    self.sse41 = true;
                    self.avx = true;
                }
                "avx2" | "avx512f" | "avx512cd" | "avx512dq" | "avx512bw" | "avx512vl"
                | "avx512ifma" | "avx512vbmi" | "avx512vbmi2" | "avx512vnni" | "avx512bitalg"
                | "avx512vpopcntdq" | "avx512bf16" | "avx512fp16" | "avx512er" | "avx512pf"
                | "avx512vp2intersect" | "avx512bmm" => {
                    self.simd = true;
                    self.sse41 = true;
                    self.avx = true;
                    self.ymm = true;
                }
                "fma" => {
                    self.simd = true;
                    self.sse41 = true;
                    self.avx = true;
                    self.fma = true;
                }
                "movbe" => self.movbe = true,
                "cx16" => self.cx16 = true,
                "arch=x86-64" => {
                    self.simd = true;
                }
                "arch=x86-64-v2" | "arch=nehalem" | "arch=westmere" | "arch=silvermont"
                | "arch=goldmont" | "arch=tremont" => {
                    self.simd = true;
                    self.sse41 = true;
                    self.cx16 = true;
                }
                "arch=sandybridge" | "arch=ivybridge" => {
                    self.simd = true;
                    self.sse41 = true;
                    self.avx = true;
                    self.cx16 = true;
                }
                "arch=x86-64-v3"
                | "arch=x86-64-v4"
                | "arch=haswell"
                | "arch=broadwell"
                | "arch=skylake"
                | "arch=skylake-avx512"
                | "arch=cascadelake"
                | "arch=cooperlake"
                | "arch=icelake-client"
                | "arch=icelake-server"
                | "arch=tigerlake"
                | "arch=sapphirerapids"
                | "arch=emeraldrapids"
                | "arch=graniterapids"
                | "arch=alderlake"
                | "arch=raptorlake"
                | "arch=meteorlake"
                | "arch=arrowlake"
                | "arch=znver1"
                | "arch=znver2"
                | "arch=znver3"
                | "arch=znver4"
                | "arch=znver5" => {
                    self = Self::V3;
                }
                _ => {}
            }
        }
        for feat in features {
            match feat.as_str() {
                "general-regs-only" | "no-sse" | "no-sse2" => {
                    self.simd = false;
                }
                "no-sse4" | "no-sse4.1" => {
                    self.sse41 = false;
                }
                "no-avx" => {
                    self.avx = false;
                }
                "no-avx2" => {
                    self.ymm = false;
                }
                "no-fma" => {
                    self.fma = false;
                }
                "no-movbe" => {
                    self.movbe = false;
                }
                "no-cx16" => {
                    self.cx16 = false;
                }
                _ => {}
            }
        }
        self.normalized()
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
