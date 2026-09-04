//! x86 CPU tuning model: measured microarchitectural facts that drive
//! target-dependent code-generation decisions.
//!
//! # Design
//!
//! GCC (`x86-tune.def`) and LLVM (`X86.td` `Tuning*` features) both encode
//! tuning as *boolean feature bits* whose meaning is hidden in the pass that
//! consumes them, and both assign those bits by *lineage* ("every core after
//! Haswell inherits Haswell's tuning") rather than by measurement.  Two
//! consequences visible in their output today (verified with
//! `scripts/godbolt.py`, GCC 16.2 / Clang 23.1 / ICX; see
//! `docs/CPU_MODEL_AUDIT.md`):
//!
//! * GCC 16.2 `-march=skylake` emits `xorl %eax,%eax; tzcntq %rdi,%rax`
//!   although uops.info measures **no** output dependency for TZCNT/LZCNT on
//!   Skylake (`TZCNT_R64_R64`, lat 1→1 = 0); only POPCNT retains it there.
//!   The single `X86_TUNE_AVOID_FALSE_DEP_FOR_BMI` bit cannot express that.
//! * LLVM keeps `TuningSlow3OpsLEA` on Ice Lake and Alder Lake although
//!   uops.info shows the 3-component LEA moved from `p1` (throughput 1.0)
//!   to `p0156` (throughput 0.31 / 0.20) there, i.e. it is no longer slow.
//!
//! This module therefore stores **numbers** (latencies, µop counts, port
//! throughputs, structure sizes) per microarchitecture, one row per
//! measurement, and derives the decisions from the numbers.  Every field
//! carries its provenance:
//!
//! * `[uops.info]` — measured by the uops.info harness
//!   (Abel & Reineke, "uops.info: Characterizing Latency, Throughput, and
//!   Port Usage of Instructions on Intel Microarchitectures", ASPLOS 2019),
//!   instruction page named in the comment.  Values were re-read from the
//!   live site during the session that introduced the field.
//! * `[Agner]` — Agner Fog, "Instruction tables" / "The microarchitecture of
//!   Intel, AMD and VIA CPUs" (2024 edition).
//! * `[Intel ORM]`, `[AMD SOG]` — vendor optimisation manuals.
//! * `[cpuid]` — architectural, not a measurement.
//!
//! A field with no provenance line is not allowed in this file.
//!
//! # Scope
//!
//! Sandy Bridge and later Intel Core, Alder Lake/Raptor Lake hybrid parts
//! (P-core numbers; the E-core column is noted where uops.info has it), and
//! AMD Zen 1–5.  Older cores and Atom-class parts resolve to the
//! conservative `Generic` row.
//!
//! # Policy for `Generic`
//!
//! `Generic` is what `-mtune` defaults to when neither `-march` nor
//! `-mtune` names a CPU.  It must be *safe* on every supported core: where
//! a work-around costs one eliminated µop on the cores that do not need it
//! but saves a multi-cycle dependency chain on the cores that do, the
//! work-around is on.  Where the choice is a pure win everywhere
//! (e.g. SHLX over `SHL r,cl`), it is on.
//!
//! # Extending the model
//!
//! Add the measured number, its provenance, and a unit test that pins it.
//! Then derive the decision in a `fn` on [`X86Tune`] so callers never read
//! raw numbers.  Never add a bare `bool` that a pass "interprets".

use std::sync::OnceLock;

/// Microarchitectures the tuning model distinguishes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum X86Cpu {
    /// Conservative blend: safe on every Sandy Bridge+ / Zen core.
    Generic,
    SandyBridge,
    IvyBridge,
    Haswell,
    Broadwell,
    /// Skylake client and its derivatives (Kaby/Coffee/Comet Lake) share
    /// the core; they are the same row.
    Skylake,
    /// Skylake-SP / Cascade Lake / Cooper Lake (Skylake core + AVX-512).
    SkylakeAvx512,
    /// Ice Lake client/server, Tiger Lake, Rocket Lake (Sunny/Cypress Cove).
    IceLake,
    /// Alder Lake / Raptor Lake P-core (Golden/Raptor Cove).  Decisions are
    /// made for the P-core, which runs the hot code under the default
    /// scheduler; Gracemont numbers are noted in the field comments.
    AlderLake,
    /// Sapphire/Emerald Rapids (Golden Cove server + AVX-512).
    SapphireRapids,
    /// Arrow Lake / Lunar Lake (Lion Cove P-core).
    ArrowLake,
    Znver1,
    Znver2,
    Znver3,
    Znver4,
    Znver5,
}

/// One tuning row.  Latencies in cycles; reciprocal throughputs ×100 so the
/// row stays integer and `Copy`.  Zero means "not available on this core".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct X86Tune {
    pub cpu: X86Cpu,
    /// GCC/Clang-compatible spelling used in diagnostics.
    pub name: &'static str,

    // ------------------------------------------------------------------
    // False output dependencies.  `[uops.info]` "Latency operand 1 → 1"
    // of POPCNT_R64_R64 / LZCNT_R64_R64 / TZCNT_R64_R64: 3 cycles where the
    // destination is (wrongly) read, 0 where it is not.
    // ------------------------------------------------------------------
    /// POPCNT reads its destination: SNB, IVB, HSW, BDW, SKL, SKX, CLX.
    /// Fixed on ICL/TGL/RKL/ADL-P/ADL-E and on every Zen.
    pub popcnt_false_dep: bool,
    /// LZCNT/TZCNT read their destination: SNB, IVB, HSW, BDW.  Fixed on
    /// SKL and later (measured 0 on SKL/SKX/CLX/ICL/ADL) and on every Zen.
    pub lzcnt_tzcnt_false_dep: bool,

    // ------------------------------------------------------------------
    // Shifts by CL.  `[uops.info]` SHL_R64_CL "Number of µops" and measured
    // unrolled throughput; SHLX_R64_R64_R64 is 1 µop with rTP 0.5 (Intel
    // p06) / 0.25–0.33 (Zen) everywhere it exists.
    // ------------------------------------------------------------------
    /// µops of `SHL r64, CL`: 3 on SNB..CLX (rTP 1.5–2.0), 2 on ICL/TGL/
    /// RKL/ADL-P (rTP 1.0), 1 on every Zen (rTP 0.25–0.5).
    pub shift_cl_uops: u8,

    // ------------------------------------------------------------------
    // LEA.  `[uops.info]` LEA_B_I_D8_R64 (base+index+disp8): SNB..SKL
    // execute it on p1 only with rTP 1.0 (the 3-cycle "slow LEA" [Agner]);
    // ICL runs it on p0156 rTP 0.31, ADL-P on p0156B rTP 0.20 → 1 cycle.
    // Zen: 2 cycles for 3 components [Agner, Zen tables], rTP 0.5.
    // ------------------------------------------------------------------
    /// Latency of a 3-component LEA (base + index + displacement).
    pub lea3_latency: u8,

    // ------------------------------------------------------------------
    // CMOV.  `[uops.info]` CMOVB_R64_R64: 2 µops (p015+p05 / p0156+p06),
    // lat 2 on SNB/IVB/HSW; 1 µop lat 1 from BDW on and on every Zen.
    // ------------------------------------------------------------------
    pub cmov_uops: u8,
    pub cmov_latency: u8,

    // ------------------------------------------------------------------
    // Integer multiply / divide.  `[uops.info]` IMUL_R64_R64 latency 3 on
    // every P-core and Zen (5 on ADL-E).  DIV_R64 latency (operand 1→3,
    // dividend→quotient) and measured rTP: SNB 29/22.6, IVB 30/22.4,
    // HSW 30/21, BDW 30/21, SKL 34/21, SKX 34/21, CLX 34/21, ICL 18/10,
    // TGL 18/10, RKL 18/10, ADL-P 18/10, ADL-E 13/6, Zen+ 10/14,
    // Zen2 10/14, Zen3 10/7, Zen4 10/7, Zen5 10/7.
    // ------------------------------------------------------------------
    pub imul64_latency: u8,
    pub div64_latency: u8,
    /// Reciprocal throughput of DIV r64 ×100.
    pub div64_rtp_x100: u16,

    // ------------------------------------------------------------------
    // Floating point.  `[Agner]` VFMADD231PS ymm: HSW/BDW 5, SKL..SKX 4,
    // ICL 4, ADL-P 4, Zen1/2 5, Zen3+ 4; not present on SNB/IVB.
    // VADDPS ymm: SNB..BDW 3, SKL..RKL 4 (executed on the FMA unit),
    // ADL-P 3 (dedicated adder), Zen 3.  VMULPS ymm: SNB..HSW 5, BDW 3,
    // SKL+ 4, Zen 3.  256-bit FMA-capable pipes: 2 on every listed core
    // except Zen1 (2×128 halves → counted as 1).
    // ------------------------------------------------------------------
    pub fma_latency: u8,
    pub fadd_latency: u8,
    pub fmul_latency: u8,
    pub fma_pipes: u8,

    // ------------------------------------------------------------------
    // Memory.  `[cpuid]` ERMS (Enhanced REP MOVSB): IVB+ Intel, all Zen.
    // FSRM (Fast Short REP MOV): ICL+ Intel, Zen4+.  `[Agner]` SNB/IVB
    // execute a 32-byte load as two 16-byte halves; the 256-bit load path
    // is full width from HSW on.
    // ------------------------------------------------------------------
    pub erms: bool,
    pub fsrm: bool,
    pub avx256_unaligned_split: bool,

    // ------------------------------------------------------------------
    // Out-of-order / front-end structure sizes.  `[Intel ORM]`, `[Agner
    // microarchitecture]`, `[AMD SOG]`.  Recorded for the unroll,
    // scheduling and software-pipelining cost models; no decision consumes
    // them yet because they cannot be validated without the hardware (see
    // docs/FOLLOWUP_CPU_MODEL.md).
    // ------------------------------------------------------------------
    /// Reorder-buffer entries: SNB/IVB 168, HSW/BDW 192, SKL/SKX 224,
    /// ICL/TGL/RKL 352, ADL-P/SPR 512, ARL 576, Zen1 192, Zen2 224,
    /// Zen3 256, Zen4 320, Zen5 448.
    pub rob_entries: u16,
    /// Loop-stream-detector capacity in µops (0 = none / disabled by
    /// microcode): SNB/IVB 28, HSW/BDW 56, SKL/SKX/CLX 0 (erratum
    /// SKL150), ICL 0 (disabled), ADL-P 192, Zen 0.
    pub lsd_uops: u16,
    /// Decoded-µop cache capacity: SNB..SKX 1536, ICL 2304, ADL-P 4096,
    /// ARL 4096, Zen1 2048, Zen2/3 4096, Zen4 6912, Zen5 6144.
    pub uop_cache_uops: u16,
    /// Skylake-derived cores (SKL/KBL/CFL/CML/SKX/CLX) lose the DSB for a
    /// 32-byte window containing a JCC that touches the boundary after the
    /// 2019 microcode update (Intel JCC erratum, white paper 341810).
    /// Recorded for a future `-mbranches-within-32B-boundaries` default.
    pub jcc_erratum: bool,
}

impl X86Cpu {
    /// Map a `-march=`/`-mtune=` spelling (GCC/Clang vocabulary) to a row.
    /// Returns `None` for names the driver does not know at all; names it
    /// knows but has no dedicated row for resolve to the closest core.
    pub fn from_name(name: &str) -> Option<X86Cpu> {
        use X86Cpu::*;
        Some(match name {
            "generic" | "x86-64" | "x86-64-v2" | "x86-64-v3" | "x86-64-v4" | "nocona"
            | "core2" | "nehalem" | "westmere" | "silvermont" | "goldmont"
            | "goldmont-plus" | "tremont" | "bonnell" | "atom" | "k8" | "opteron"
            | "barcelona" | "bdver1" | "bdver2" | "bdver3" | "bdver4" | "btver1"
            | "btver2" | "knl" | "knm" | "intel" => Generic,
            "sandybridge" | "corei7-avx" => SandyBridge,
            "ivybridge" | "core-avx-i" => IvyBridge,
            "haswell" | "core-avx2" => Haswell,
            "broadwell" => Broadwell,
            "skylake" | "kabylake" | "coffeelake" | "cometlake" | "amberlake"
            | "whiskeylake" => Skylake,
            "skylake-avx512" | "cascadelake" | "cooperlake" | "cannonlake" => SkylakeAvx512,
            "icelake-client" | "icelake-server" | "tigerlake" | "rocketlake" => IceLake,
            "alderlake" | "raptorlake" | "raptor-lake" | "meteorlake" | "gracemont"
            | "sierraforest" | "grandridge" | "clearwaterforest" => AlderLake,
            "sapphirerapids" | "emeraldrapids" | "graniterapids" | "graniterapids-d" => {
                SapphireRapids
            }
            "arrowlake" | "arrowlake-s" | "lunarlake" | "pantherlake" | "wildcatlake" => {
                ArrowLake
            }
            "znver1" => Znver1,
            "znver2" => Znver2,
            "znver3" => Znver3,
            "znver4" => Znver4,
            "znver5" => Znver5,
            _ => return None,
        })
    }

    /// Identify the host core from CPUID family/model (leaf 1) for
    /// `-march=native` / `-mtune=native`.
    #[cfg(target_arch = "x86_64")]
    pub fn detect_host() -> X86Cpu {
        // CPUID is unprivileged and available on every x86-64 CPU (the
        // intrinsic is safe since Rust 1.86).
        let vendor = std::arch::x86_64::__cpuid(0);
        let leaf1 = std::arch::x86_64::__cpuid(1);
        let vendor_bytes = [
            vendor.ebx.to_le_bytes(),
            vendor.edx.to_le_bytes(),
            vendor.ecx.to_le_bytes(),
        ]
        .concat();
        let is_amd = &vendor_bytes[..] == b"AuthenticAMD";
        let is_intel = &vendor_bytes[..] == b"GenuineIntel";
        let eax = leaf1.eax;
        let family = ((eax >> 8) & 0xF) + ((eax >> 20) & 0xFF);
        let model = ((eax >> 4) & 0xF) | ((eax >> 12) & 0xF0);
        Self::from_signature(is_intel, is_amd, family, model)
    }

    /// Pure family/model decoder behind [`Self::detect_host`], separated so
    /// it can be unit-tested on any host.  Model numbers from Intel SDM
    /// Vol. 4 Ch. 2 and the Linux `intel-family.h` / AMD cpu tables.
    /// Parts newer than the table resolve to the newest known core of their
    /// vendor line; unknown vendors resolve to `Generic`.
    pub fn from_signature(is_intel: bool, is_amd: bool, family: u32, model: u32) -> X86Cpu {
        use X86Cpu::*;
        if is_intel && family == 6 {
            return match model {
                0x2A | 0x2D => SandyBridge,
                0x3A | 0x3E => IvyBridge,
                0x3C | 0x3F | 0x45 | 0x46 => Haswell,
                0x3D | 0x47 | 0x4F | 0x56 => Broadwell,
                0x4E | 0x5E | 0x8E | 0x9E | 0xA5 | 0xA6 => Skylake,
                // 06_55H: stepping <5 SKX, 5–6 CLX, ≥10 CPX — same core.
                0x55 | 0x66 => SkylakeAvx512,
                0x7D | 0x7E | 0x6A | 0x6C | 0x8C | 0x8D | 0xA7 => IceLake,
                0x97 | 0x9A | 0xB7 | 0xBA | 0xBF | 0xBE | 0xAA | 0xAC | 0xB5 | 0xDD => {
                    AlderLake
                }
                0x8F | 0xCF | 0xAD | 0xAE => SapphireRapids,
                0xC6 | 0xC5 | 0xBD | 0xCC => ArrowLake,
                m if m > 0xC6 => ArrowLake,
                _ => Generic,
            };
        }
        if is_amd {
            return match family {
                0x17 => match model {
                    0x00..=0x2F => Znver1,
                    _ => Znver2,
                },
                0x19 => match model {
                    0x10..=0x1F | 0x60..=0x7F | 0xA0..=0xAF => Znver4,
                    _ => Znver3,
                },
                f if f >= 0x1A => Znver5,
                _ => Generic,
            };
        }
        Generic
    }

    /// The tuning row for this core.
    pub const fn tune(self) -> X86Tune {
        use X86Cpu::*;
        const fn snb_like(cpu: X86Cpu, name: &'static str, erms: bool) -> X86Tune {
            X86Tune {
                cpu,
                name,
                popcnt_false_dep: true,
                lzcnt_tzcnt_false_dep: true,
                shift_cl_uops: 3,
                lea3_latency: 3,
                cmov_uops: 2,
                cmov_latency: 2,
                imul64_latency: 3,
                div64_latency: 30,
                div64_rtp_x100: 2260,
                fma_latency: 0,
                fadd_latency: 3,
                fmul_latency: 5,
                fma_pipes: 0,
                erms,
                fsrm: false,
                avx256_unaligned_split: true,
                rob_entries: 168,
                lsd_uops: 28,
                uop_cache_uops: 1536,
                jcc_erratum: false,
            }
        }
        match self {
            SandyBridge => snb_like(SandyBridge, "sandybridge", false),
            IvyBridge => snb_like(IvyBridge, "ivybridge", true),
            Haswell => X86Tune {
                cpu: Haswell,
                name: "haswell",
                popcnt_false_dep: true,
                lzcnt_tzcnt_false_dep: true,
                shift_cl_uops: 3,
                lea3_latency: 3,
                cmov_uops: 2,
                cmov_latency: 2,
                imul64_latency: 3,
                div64_latency: 30,
                div64_rtp_x100: 2100,
                fma_latency: 5,
                fadd_latency: 3,
                fmul_latency: 5,
                fma_pipes: 2,
                erms: true,
                fsrm: false,
                avx256_unaligned_split: false,
                rob_entries: 192,
                lsd_uops: 56,
                uop_cache_uops: 1536,
                jcc_erratum: false,
            },
            Broadwell => X86Tune {
                cpu: Broadwell,
                name: "broadwell",
                cmov_uops: 1,
                cmov_latency: 1,
                fmul_latency: 3,
                ..Haswell.tune()
            },
            Skylake => X86Tune {
                cpu: Skylake,
                name: "skylake",
                lzcnt_tzcnt_false_dep: false,
                div64_latency: 34,
                fma_latency: 4,
                fadd_latency: 4,
                fmul_latency: 4,
                rob_entries: 224,
                lsd_uops: 0,
                jcc_erratum: true,
                ..Broadwell.tune()
            },
            SkylakeAvx512 => X86Tune {
                cpu: SkylakeAvx512,
                name: "skylake-avx512",
                ..Skylake.tune()
            },
            IceLake => X86Tune {
                cpu: IceLake,
                name: "icelake-client",
                popcnt_false_dep: false,
                shift_cl_uops: 2,
                lea3_latency: 1,
                div64_latency: 18,
                div64_rtp_x100: 1000,
                fsrm: true,
                rob_entries: 352,
                lsd_uops: 0,
                uop_cache_uops: 2304,
                jcc_erratum: false,
                ..Skylake.tune()
            },
            AlderLake => X86Tune {
                cpu: AlderLake,
                name: "alderlake",
                fadd_latency: 3,
                rob_entries: 512,
                lsd_uops: 192,
                uop_cache_uops: 4096,
                ..IceLake.tune()
            },
            SapphireRapids => X86Tune {
                cpu: SapphireRapids,
                name: "sapphirerapids",
                ..AlderLake.tune()
            },
            ArrowLake => X86Tune {
                cpu: ArrowLake,
                name: "arrowlake",
                rob_entries: 576,
                ..AlderLake.tune()
            },
            Znver1 => X86Tune {
                cpu: Znver1,
                name: "znver1",
                popcnt_false_dep: false,
                lzcnt_tzcnt_false_dep: false,
                shift_cl_uops: 1,
                lea3_latency: 2,
                cmov_uops: 1,
                cmov_latency: 1,
                imul64_latency: 3,
                div64_latency: 10,
                div64_rtp_x100: 1400,
                fma_latency: 5,
                fadd_latency: 3,
                fmul_latency: 3,
                fma_pipes: 1,
                erms: true,
                fsrm: false,
                avx256_unaligned_split: false,
                rob_entries: 192,
                lsd_uops: 0,
                uop_cache_uops: 2048,
                jcc_erratum: false,
            },
            Znver2 => X86Tune {
                cpu: Znver2,
                name: "znver2",
                fma_pipes: 2,
                rob_entries: 224,
                uop_cache_uops: 4096,
                ..Znver1.tune()
            },
            Znver3 => X86Tune {
                cpu: Znver3,
                name: "znver3",
                div64_rtp_x100: 700,
                fma_latency: 4,
                rob_entries: 256,
                ..Znver2.tune()
            },
            Znver4 => X86Tune {
                cpu: Znver4,
                name: "znver4",
                fsrm: true,
                rob_entries: 320,
                uop_cache_uops: 6912,
                ..Znver3.tune()
            },
            Znver5 => X86Tune {
                cpu: Znver5,
                name: "znver5",
                rob_entries: 448,
                uop_cache_uops: 6144,
                ..Znver4.tune()
            },
            // Generic: the conservative envelope over every row above.  Each
            // field takes the value that is never *harmful*: work-arounds on,
            // latencies at the slow end, structure sizes at the small end.
            Generic => X86Tune {
                cpu: Generic,
                name: "generic",
                popcnt_false_dep: true,
                lzcnt_tzcnt_false_dep: true,
                shift_cl_uops: 3,
                lea3_latency: 3,
                cmov_uops: 2,
                cmov_latency: 2,
                imul64_latency: 3,
                div64_latency: 34,
                div64_rtp_x100: 2260,
                fma_latency: 5,
                fadd_latency: 4,
                fmul_latency: 5,
                fma_pipes: 2,
                erms: false,
                fsrm: false,
                avx256_unaligned_split: false,
                rob_entries: 168,
                lsd_uops: 0,
                uop_cache_uops: 1536,
                jcc_erratum: false,
            },
        }
    }

    /// Every row, for exhaustive tests and `LCCC_DUMP_TUNE=all`.
    pub const ALL: [X86Cpu; 16] = [
        X86Cpu::Generic,
        X86Cpu::SandyBridge,
        X86Cpu::IvyBridge,
        X86Cpu::Haswell,
        X86Cpu::Broadwell,
        X86Cpu::Skylake,
        X86Cpu::SkylakeAvx512,
        X86Cpu::IceLake,
        X86Cpu::AlderLake,
        X86Cpu::SapphireRapids,
        X86Cpu::ArrowLake,
        X86Cpu::Znver1,
        X86Cpu::Znver2,
        X86Cpu::Znver3,
        X86Cpu::Znver4,
        X86Cpu::Znver5,
    ];
}

impl X86Tune {
    /// The row used when no tuning information is available at all.
    pub const GENERIC: X86Tune = X86Cpu::Generic.tune();

    // ------------------------------------------------------------------
    // Derived decisions.  Callers use these, never the raw numbers.
    // ------------------------------------------------------------------

    /// Emit `xor %d,%d` before `popcnt %s,%d` (d ≠ s)?  Only where the
    /// destination is architecturally read.  On other cores the xor is a
    /// zero idiom (no execution µop) but still costs a front-end slot and
    /// two to three bytes per instance.
    #[inline]
    pub fn break_popcnt_dep(&self) -> bool {
        self.popcnt_false_dep
    }

    /// Emit `xor %d,%d` before `lzcnt`/`tzcnt %s,%d` (d ≠ s)?
    #[inline]
    pub fn break_lzcnt_tzcnt_dep(&self) -> bool {
        self.lzcnt_tzcnt_false_dep
    }

    /// Select `shlx/shrx/sarx` over `shl/shr/sar r, %cl` when BMI2 is
    /// available and the count is already in `%rcx`.  The VEX form is 1 µop
    /// on every core that has it, so it wins wherever the legacy form is
    /// more than one µop (all Intel cores); on Zen (1 µop each) the legacy
    /// form is 2–3 bytes shorter and otherwise identical, so it is kept.
    #[inline]
    pub fn prefer_shlx(&self, bmi2: bool) -> bool {
        bmi2 && self.shift_cl_uops > 1
    }

    /// Use `shlx` when doing so removes a register copy (count not in
    /// `%rcx`, or the result must not clobber the source).  A saved `mov`
    /// is a saved µop/slot on every core, including Zen.
    #[inline]
    pub fn shlx_saves_move(&self, bmi2: bool) -> bool {
        bmi2
    }

    /// Independent accumulators needed to saturate the FMA pipes in a
    /// reduction: `latency × pipes` (Little's law).  0 when the core has no
    /// FMA (SNB/IVB) so the caller falls back to the FADD figure.
    #[inline]
    pub fn fma_reduction_accumulators(&self) -> u8 {
        self.fma_latency.saturating_mul(self.fma_pipes.max(1))
    }

    /// Same for a pure add reduction (two 256-bit adders on every row).
    #[inline]
    pub fn fadd_reduction_accumulators(&self) -> u8 {
        self.fadd_latency.saturating_mul(2)
    }

    /// Human-readable dump (one `key=value` per line) for
    /// `LCCC_DUMP_TUNE=1` and the regression tests.
    pub fn dump(&self) -> String {
        let mut s = String::with_capacity(1024);
        let mut kv = |k: &str, v: String| {
            s.push_str(k);
            s.push('=');
            s.push_str(&v);
            s.push('\n');
        };
        kv("tune", self.name.to_string());
        kv("popcnt_false_dep", self.popcnt_false_dep.to_string());
        kv("lzcnt_tzcnt_false_dep", self.lzcnt_tzcnt_false_dep.to_string());
        kv("shift_cl_uops", self.shift_cl_uops.to_string());
        kv("lea3_latency", self.lea3_latency.to_string());
        kv("cmov_uops", self.cmov_uops.to_string());
        kv("cmov_latency", self.cmov_latency.to_string());
        kv("imul64_latency", self.imul64_latency.to_string());
        kv("div64_latency", self.div64_latency.to_string());
        kv("div64_rtp_x100", self.div64_rtp_x100.to_string());
        kv("fma_latency", self.fma_latency.to_string());
        kv("fadd_latency", self.fadd_latency.to_string());
        kv("fmul_latency", self.fmul_latency.to_string());
        kv("fma_pipes", self.fma_pipes.to_string());
        kv("erms", self.erms.to_string());
        kv("fsrm", self.fsrm.to_string());
        kv("avx256_unaligned_split", self.avx256_unaligned_split.to_string());
        kv("rob_entries", self.rob_entries.to_string());
        kv("lsd_uops", self.lsd_uops.to_string());
        kv("uop_cache_uops", self.uop_cache_uops.to_string());
        kv("jcc_erratum", self.jcc_erratum.to_string());
        kv(
            "derived.fma_reduction_accumulators",
            self.fma_reduction_accumulators().to_string(),
        );
        kv(
            "derived.fadd_reduction_accumulators",
            self.fadd_reduction_accumulators().to_string(),
        );
        s
    }
}

impl Default for X86Tune {
    fn default() -> Self {
        X86Tune::GENERIC
    }
}

/// Resolve the effective tuning row from the driver's `-march` / `-mtune`
/// spellings.  Precedence follows GCC: `-mtune` wins; otherwise `-march`
/// names the core; otherwise `generic`.  `native` resolves through CPUID on
/// x86-64 hosts and to `generic` elsewhere.
pub fn resolve(march: Option<&str>, mtune: Option<&str>) -> X86Tune {
    fn one(name: &str) -> Option<X86Cpu> {
        if name == "native" {
            #[cfg(target_arch = "x86_64")]
            {
                return Some(X86Cpu::detect_host());
            }
            #[cfg(not(target_arch = "x86_64"))]
            {
                return Some(X86Cpu::Generic);
            }
        }
        X86Cpu::from_name(name)
    }
    let cpu = mtune
        .and_then(one)
        .or_else(|| march.and_then(one))
        .unwrap_or(X86Cpu::Generic);
    cpu.tune()
}

// ----------------------------------------------------------------------
// Process-wide active model.  The middle-end passes are free functions
// that receive no target context (see passes::run_passes); like
// `vectorize::set_x86_fma_enabled` they read a global that the driver sets
// once per compilation.  Backend code receives the model through
// `CodegenOptions` instead and must not use this accessor.
// ----------------------------------------------------------------------

static ACTIVE: OnceLock<std::sync::RwLock<X86Tune>> = OnceLock::new();

fn active_cell() -> &'static std::sync::RwLock<X86Tune> {
    ACTIVE.get_or_init(|| std::sync::RwLock::new(X86Tune::GENERIC))
}

/// Install the model for the current compilation (driver only).
pub fn set_active(t: X86Tune) {
    *active_cell().write().unwrap_or_else(|e| e.into_inner()) = t;
}

/// The model installed by [`set_active`], `GENERIC` before the driver runs.
pub fn active() -> X86Tune {
    *active_cell().read().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_row_is_reachable_by_name() {
        for cpu in X86Cpu::ALL {
            let t = cpu.tune();
            assert_eq!(t.cpu, cpu, "row {:?} reports wrong cpu", cpu);
            assert_eq!(
                X86Cpu::from_name(t.name),
                Some(cpu),
                "row name {:?} does not round-trip",
                t.name
            );
        }
    }

    /// Pins the uops.info "latency operand 1 → 1" measurements that the
    /// false-dependency decision rests on.  If a row here changes, the
    /// commit must cite a new measurement.
    #[test]
    fn false_dependency_rows_match_uops_info() {
        use X86Cpu::*;
        let popcnt_dep: &[X86Cpu] =
            &[SandyBridge, IvyBridge, Haswell, Broadwell, Skylake, SkylakeAvx512];
        let lzcnt_dep: &[X86Cpu] = &[SandyBridge, IvyBridge, Haswell, Broadwell];
        for cpu in X86Cpu::ALL {
            if cpu == Generic {
                continue;
            }
            let t = cpu.tune();
            assert_eq!(t.popcnt_false_dep, popcnt_dep.contains(&cpu), "popcnt {:?}", cpu);
            assert_eq!(t.lzcnt_tzcnt_false_dep, lzcnt_dep.contains(&cpu), "lzcnt {:?}", cpu);
        }
        // Skylake is the row GCC 16.2 gets wrong (xor before tzcnt).
        assert!(Skylake.tune().popcnt_false_dep && !Skylake.tune().lzcnt_tzcnt_false_dep);
    }

    #[test]
    fn shift_cl_uops_match_uops_info() {
        use X86Cpu::*;
        for cpu in [SandyBridge, IvyBridge, Haswell, Broadwell, Skylake, SkylakeAvx512] {
            assert_eq!(cpu.tune().shift_cl_uops, 3, "{:?}", cpu);
        }
        for cpu in [IceLake, AlderLake, SapphireRapids, ArrowLake] {
            assert_eq!(cpu.tune().shift_cl_uops, 2, "{:?}", cpu);
        }
        for cpu in [Znver1, Znver2, Znver3, Znver4, Znver5] {
            assert_eq!(cpu.tune().shift_cl_uops, 1, "{:?}", cpu);
            assert!(!cpu.tune().prefer_shlx(true));
            assert!(cpu.tune().shlx_saves_move(true));
        }
        for cpu in [Haswell, Skylake, IceLake, AlderLake, Generic] {
            assert!(cpu.tune().prefer_shlx(true));
            assert!(!cpu.tune().prefer_shlx(false));
        }
    }

    #[test]
    fn lea_and_cmov_and_div_rows() {
        use X86Cpu::*;
        for cpu in [SandyBridge, Haswell, Skylake, SkylakeAvx512] {
            assert_eq!(cpu.tune().lea3_latency, 3);
        }
        for cpu in [IceLake, AlderLake, ArrowLake] {
            assert_eq!(cpu.tune().lea3_latency, 1);
        }
        for cpu in [SandyBridge, IvyBridge, Haswell] {
            assert_eq!(cpu.tune().cmov_uops, 2);
        }
        for cpu in [Broadwell, Skylake, IceLake, AlderLake, Znver1, Znver5] {
            assert_eq!(cpu.tune().cmov_uops, 1);
        }
        assert!(Skylake.tune().div64_latency > IceLake.tune().div64_latency);
        assert!(IceLake.tune().div64_latency > Znver3.tune().div64_latency);
        assert_eq!(Znver2.tune().div64_rtp_x100, 1400);
        assert_eq!(Znver3.tune().div64_rtp_x100, 700);
    }

    #[test]
    fn generic_is_the_conservative_envelope() {
        let g = X86Tune::GENERIC;
        for cpu in X86Cpu::ALL {
            let t = cpu.tune();
            // Work-arounds: generic applies every one that any row needs.
            assert!(g.popcnt_false_dep >= t.popcnt_false_dep);
            assert!(g.lzcnt_tzcnt_false_dep >= t.lzcnt_tzcnt_false_dep);
            // Capacities: generic never assumes more than the smallest core.
            assert!(g.rob_entries <= t.rob_entries, "{:?}", cpu);
            assert!(g.uop_cache_uops <= t.uop_cache_uops, "{:?}", cpu);
            // Optional fast paths: generic never assumes them.
            assert!(!g.erms && !g.fsrm);
        }
    }

    #[test]
    fn resolve_precedence_and_native() {
        assert_eq!(resolve(None, None).cpu, X86Cpu::Generic);
        assert_eq!(resolve(Some("skylake"), None).cpu, X86Cpu::Skylake);
        assert_eq!(resolve(Some("skylake"), Some("znver3")).cpu, X86Cpu::Znver3);
        assert_eq!(resolve(Some("x86-64-v3"), None).cpu, X86Cpu::Generic);
        assert_eq!(resolve(None, Some("raptorlake")).cpu, X86Cpu::AlderLake);
        assert_eq!(resolve(None, Some("no-such-cpu")).cpu, X86Cpu::Generic);
        let n = resolve(None, Some("native"));
        assert!(X86Cpu::ALL.contains(&n.cpu));
    }

    #[test]
    fn cpuid_signature_decoder() {
        use X86Cpu::*;
        let intel = |m| X86Cpu::from_signature(true, false, 6, m);
        assert_eq!(intel(0x2A), SandyBridge);
        assert_eq!(intel(0x3A), IvyBridge);
        assert_eq!(intel(0x3C), Haswell);
        assert_eq!(intel(0x4E), Skylake);
        assert_eq!(intel(0x9E), Skylake); // Coffee Lake
        assert_eq!(intel(0x55), SkylakeAvx512);
        assert_eq!(intel(0x7E), IceLake);
        assert_eq!(intel(0x97), AlderLake);
        assert_eq!(intel(0xB7), AlderLake); // Raptor Lake (i7-14700KF)
        assert_eq!(intel(0x8F), SapphireRapids);
        assert_eq!(intel(0xC6), ArrowLake);
        let amd = |f, m| X86Cpu::from_signature(false, true, f, m);
        assert_eq!(amd(0x17, 0x01), Znver1);
        assert_eq!(amd(0x17, 0x71), Znver2);
        assert_eq!(amd(0x19, 0x21), Znver3);
        assert_eq!(amd(0x19, 0x61), Znver4);
        assert_eq!(amd(0x1A, 0x44), Znver5);
        assert_eq!(X86Cpu::from_signature(false, false, 6, 0x97), Generic);
    }

    #[test]
    fn reduction_accumulators_follow_littles_law() {
        assert_eq!(X86Cpu::Haswell.tune().fma_reduction_accumulators(), 10);
        assert_eq!(X86Cpu::Skylake.tune().fma_reduction_accumulators(), 8);
        assert_eq!(X86Cpu::Znver1.tune().fma_reduction_accumulators(), 5);
        assert_eq!(X86Cpu::Znver3.tune().fma_reduction_accumulators(), 8);
        assert_eq!(X86Cpu::SandyBridge.tune().fma_reduction_accumulators(), 0);
        assert_eq!(X86Cpu::AlderLake.tune().fadd_reduction_accumulators(), 6);
    }

    #[test]
    fn dump_is_stable_and_complete() {
        let d = X86Cpu::AlderLake.tune().dump();
        assert!(d.starts_with("tune=alderlake\n"));
        assert!(d.contains("popcnt_false_dep=false\n"));
        assert!(d.contains("shift_cl_uops=2\n"));
        assert!(d.contains("lsd_uops=192\n"));
        assert!(d.contains("derived.fma_reduction_accumulators=8\n"));
    }
}
