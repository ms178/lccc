//! x86 CPU tuning model: measured microarchitectural facts that drive
//! target-dependent code-generation decisions.
//!
//! # Design
//!
//! GCC (`x86-tune.def`) and LLVM (`X86.td` `Tuning*` features) both encode
//! tuning as *boolean feature bits* whose meaning is hidden in the pass that
//! consumes them, and both assign those bits by *lineage* ("every core after
//! Haswell inherits Haswell's tuning") rather than by measurement.  Three
//! consequences visible in their output today (verified with
//! `scripts/godbolt.py`, GCC 16.2 / Clang 23.1 / ICX; see
//! `docs/CPU_MODEL_AUDIT.md`):
//!
//! * GCC 16.2 `-march=skylake` emits `xorl %eax,%eax; tzcntq %rdi,%rax`
//!   although uops.info measures **no** output dependency for TZCNT/LZCNT on
//!   Skylake (`TZCNT_R64_R64`, lat 1→1 = 0); only POPCNT retains it there.
//!   The single `X86_TUNE_AVOID_FALSE_DEP_FOR_BMI` bit cannot express that.
//! * LLVM keeps `TuningSlow3OpsLEA` on Ice Lake and Alder Lake although
//!   uops.info shows the 3-component LEA moved from `p1` (throughput 1.0,
//!   latency 3) to `p0156` (throughput 0.25 / 0.20, latency 1) there.
//! * Both treat Raptor Lake as a spelling of Alder Lake and Gracemont-only
//!   parts (Sierra Forest, Alder Lake-N) as Alder Lake P-cores, although the
//!   E-core executes a 3-component LEA in 2 cycles at throughput 1.0 and
//!   `SHL r,CL` as a single µop (uops.info ADL-E), i.e. the opposite of the
//!   P-core on both counts.
//!
//! This module therefore stores **numbers** (latencies, µop counts, port
//! throughputs, structure and cache sizes) per microarchitecture, one row
//! per measurement, and derives the decisions from the numbers.  Every
//! field carries its provenance:
//!
//! * `[uops.info]` — measured by the uops.info harness (Abel & Reineke,
//!   ASPLOS 2019), instruction page named in the comment.  Values were
//!   re-read from the live site in the session that introduced the field;
//!   `scripts/uops_info_probe.py <PAGE>` re-fetches and prints the same
//!   rows so any number here can be re-verified in seconds.
//! * `[Agner]` — Agner Fog, "Instruction tables" / "The microarchitecture
//!   of Intel, AMD and VIA CPUs" (2024 edition).
//! * `[Intel ORM]`, `[AMD SOG]` — vendor optimisation manuals.
//! * `[Intel ARK]` — product specification (cache sizes, core counts).
//! * `[glibc]` — thresholds glibc's `sysdeps/x86/dl-cacheinfo.h` derives for
//!   the same hardware; these are the most heavily validated string-op
//!   thresholds in existence and lccc reuses them rather than guessing.
//! * `[LLVM sched]` — `llvm/lib/Target/X86/X86Sched*.td` (only where no
//!   primary measurement exists; noted explicitly).
//! * `[cpuid]` — architectural, not a measurement.
//!
//! A field with no provenance line is not allowed in this file.
//!
//! # Hybrid parts (Alder Lake, Raptor Lake, Arrow Lake)
//!
//! Decisions are made for the P-core, which runs the hot code under the
//! default scheduler, but each hybrid row also carries the E-core facts
//! ([`ECoreTune`]).  The rule for derived decisions is: *when the P-core is
//! indifferent between two forms, pick the one that is not pathological on
//! the E-core*.  Sierra Forest / Alder Lake-N style E-core-only parts
//! resolve to the [`X86Cpu::Gracemont`] row instead of the P-core row.
//!
//! # Raptor Lake vs Alder Lake
//!
//! Raptor Cove is the Golden Cove core; uops.info publishes no separate
//! Raptor Lake column and every instruction latency/µop measurement of
//! ADL-P applies unchanged (verified for every instruction cited in this
//! file: the ADL-P, MTL-P and EMR columns agree except where noted).  What
//! *does* differ, and is modelled here:
//!
//! * P-core L2: 2 MiB, 16-way (ADL: 1.25 MiB, 10-way) `[Intel ARK]`.
//! * E-core cluster L2: 4 MiB (ADL: 2 MiB) `[Intel ARK]`.
//! * Shared L3: 33 MiB on the i7-14700K(F), 36 MiB on i9 (ADL: 25/30 MiB).
//! * More E-cores per die (12 on the i7-14700KF vs 4 on the i7-12700K),
//!   so a larger share of threads runs on Gracemont; the hybrid rule above
//!   therefore matters more on Raptor Lake than on Alder Lake.
//! * Higher clocks (5.6 GHz turbo vs 5.0) → the same DRAM latency costs
//!   ~12 % more cycles; recorded as `dram_latency_cycles`.
//! * Unaffected by the Gather Data Sampling (Downfall) microcode that
//!   slows VPGATHER on SKL..TGL; the ADL-P gather throughput measured by
//!   uops.info (2.67 cycles per 8-lane gather) is what Raptor Lake gets.
//!
//! # Policy for `Generic`
//!
//! `Generic` is what `-mtune` defaults to when neither `-march` nor
//! `-mtune` names a CPU.  It must be *safe* on every supported core: where
//! a work-around costs one eliminated µop on the cores that do not need it
//! but saves a multi-cycle dependency chain on the cores that do, the
//! work-around is on.  Where the choice is a pure win everywhere
//! (e.g. SHLX over `SHL r,cl`), it is on.  Where a *pessimistic* number
//! would make a transform *more* aggressive (a slow IMUL makes the
//! multiply-by-constant synthesiser emit longer LEA/SHL chains), the
//! envelope takes the value of the majority of cores, not the worst one:
//! being pessimistic there would cost µops on every P-core and every Zen.
//! Every derived decision must reproduce the pre-model default for
//! `Generic` (pinned by unit tests) so that untuned builds never change
//! behaviour when a row is refined.
//!
//! # Extending the model
//!
//! Add the measured number, its provenance, and a unit test that pins it.
//! Then derive the decision in a `fn` on [`X86Tune`] so callers never read
//! raw numbers.  Never add a bare `bool` that a pass "interprets".  A
//! field nobody consumes is a liability, not an asset: the `dump()` output
//! lists the derived decisions so `scripts/tune_oracle.py` can prove each
//! field reaches a decision.
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
    /// Alder Lake (Golden Cove P-core + Gracemont E-core).
    AlderLake,
    /// Raptor Lake (Raptor Cove P-core + Gracemont E-core).  Same core as
    /// Golden Cove; larger caches, more E-cores, higher clocks (see module
    /// docs).  Meteor Lake's Redwood Cove shares the 2 MiB L2 and resolves
    /// here as well (its only measured instruction-level delta is VMULP*
    /// latency 3 instead of 4, `[uops.info]` MTL-P).
    RaptorLake,
    /// Sapphire/Emerald Rapids (Golden Cove server + AVX-512, 2 MiB L2).
    SapphireRapids,
    /// Arrow Lake / Lunar Lake (Lion Cove P-core + Skymont E-core).
    ArrowLake,
    /// Gracemont-only parts: Alder Lake-N, Sierra Forest, Grand Ridge.
    /// Also the E-core column of the Alder/Raptor Lake hybrid rows.
    Gracemont,
    Znver1,
    Znver2,
    Znver3,
    Znver4,
    Znver5,
}

/// One cache level.  Sizes in KiB so the row stays integer and `Copy`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CacheLevel {
    pub kib: u32,
    pub ways: u8,
    /// Load-to-use latency in cycles for a hit at this level (integer
    /// load, simple addressing).
    pub latency: u8,
}

/// Cache hierarchy as seen by one hardware thread.
///
/// `[Intel ARK]` / `[AMD SOG]` for sizes and associativity, `[Agner
/// microarchitecture]` and chipsandcheese pointer-chase measurements for
/// latencies.  `l3` is the largest client die of the generation (the
/// figure GCC's `-march=native` would read from CPUID leaf 4 on the
/// flagship part); `-mtune=native` replaces all sizes with the host's
/// CPUID leaf 4 / 0x8000001D values ([`X86Cpu::detect_host_cache`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CacheModel {
    pub line_bytes: u8,
    pub l1d: CacheLevel,
    pub l2: CacheLevel,
    pub l3: CacheLevel,
    /// Approximate DRAM load-to-use latency in core cycles at the part's
    /// turbo clock (`[Intel ORM]`/`[AMD SOG]` ~80–90 ns × f_turbo).  Used
    /// only as a *scale* for prefetch-distance / tiling heuristics, never
    /// as an exact figure.
    pub dram_latency_cycles: u16,
}

/// E-core facts carried by hybrid rows.  Only fields that measurably
/// differ from the P-core are recorded; everything else is assumed to
/// follow the P-core decision.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ECoreTune {
    pub name: &'static str,
    /// `[uops.info]` LEA_B_I_D8_R64 on ADL-E / MTL-E: latency 2, rTP 1.00
    /// (a single AGU-class unit).  Skymont (ARL-E): latency 2, rTP 0.29.
    pub lea3_latency: u8,
    pub lea3_rtp_x100: u16,
    /// `[uops.info]` SHL_R64_CL on ADL-E: rTP 0.54, no microcode → 1 µop;
    /// ARL-E rTP 0.47.
    pub shift_cl_uops: u8,
    /// `[uops.info]` IMUL_R64_R64 latency 5 on ADL-E/MTL-E (3 on every
    /// P-core), 4 on ARL-E.
    pub imul64_latency: u8,
    /// `[uops.info]` DIV_R64 on ADL-E: latency 12–43; ARL-E 12–32.
    pub div64_latency: u8,
    pub div64_rtp_x100: u16,
    /// `[uops.info]` VFMADD231PS ymm: ADL-E 6, ARL-E 4.
    pub fma_latency: u8,
    /// `[Agner]` Gracemont L1D load-to-use: 3 cycles.
    pub load_latency: u8,
    /// `[Intel ARK]` L2 shared by a 4-core cluster: 2 MiB (ADL), 4 MiB (RPL).
    pub cluster_l2_kib: u32,
}

/// One tuning row.  Latencies in cycles; reciprocal throughputs ×100 so the
/// row stays integer and `Copy`.  Zero means "not available on this core".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct X86Tune {
    pub cpu: X86Cpu,
    /// GCC/Clang-compatible spelling used in diagnostics.
    pub name: &'static str,

    // ------------------------------------------------------------------
    // Front end / out-of-order machine.
    // ------------------------------------------------------------------
    /// Rename/allocation width in µops per cycle.  `[Intel ORM]`: SNB..SKX
    /// 4, ICL/TGL/RKL 5, GLC/RPC/SPR 6, LNC 8, Gracemont 5.  `[AMD SOG]`:
    /// Zen1..Zen4 dispatch 6 integer µops per cycle, Zen5 8.  Used as the
    /// throughput bound (ops / width) in cycle estimates.
    pub dispatch_width: u8,
    /// Branch-misprediction penalty in cycles (front-end restart from the
    /// µop cache).  `[Agner microarchitecture]`: SNB/IVB/HSW/BDW "15–20",
    /// SKL "16–20", ICL "~16"; chipsandcheese pointer-chase for Golden
    /// Cove 17 (µop cache) / 20+ (L1i); Gracemont ~13; `[Agner]` Zen1/2
    /// ~18, Zen3 13, Zen4 13; `[LLVM sched]` Znver5 14.  The low end of a
    /// measured range is used so that speculation budgets are never
    /// optimistic.
    pub mispredict_penalty: u8,
    /// Reorder-buffer entries `[Intel ORM]`/`[AMD SOG]`: SNB/IVB 168,
    /// HSW/BDW 192, SKL/SKX 224, ICL/TGL/RKL 352, GLC/RPC/SPR 512, LNC
    /// 576, Gracemont 256, Zen1 192, Zen2 224, Zen3 256, Zen4 320, Zen5 448.
    pub rob_entries: u16,
    /// µops the loop stream detector can lock in the µop queue (0 = none /
    /// disabled by microcode).  `[Intel ORM]`: SNB/IVB 28, HSW/BDW 56,
    /// SKL/SKX/CLX 0 (erratum SKL150), ICL 70 (re-enabled; 70-entry
    /// queue), GLC/RPC 144 (144-entry queue per thread), Gracemont 0, Zen
    /// 0 (no LSD; the op cache serves loops).
    pub lsd_uops: u16,
    /// Decoded-µop cache capacity `[Intel ORM]`/`[AMD SOG]`: SNB..SKX 1536,
    /// ICL 2304, GLC/RPC/SPR 4096, LNC 5250, Gracemont 0 (no µop cache; a
    /// 2×3-wide on-demand decoder), Zen1 2048, Zen2/3 4096, Zen4 6912,
    /// Zen5 6144.
    pub uop_cache_uops: u16,
    /// Skylake-derived cores (SKL/KBL/CFL/CML/SKX/CLX) lose the DSB for a
    /// 32-byte window containing a JCC that touches the boundary after the
    /// 2019 microcode update (Intel JCC erratum, white paper 341810).
    /// Recorded for a future `-mbranches-within-32B-boundaries` default.
    pub jcc_erratum: bool,

    // ------------------------------------------------------------------
    // False output dependencies.  `[uops.info]` "Latency operand 1 → 1"
    // of POPCNT_R64_R64 / LZCNT_R64_R64 / TZCNT_R64_R64: 3 cycles where the
    // destination is (wrongly) read, 0 where it is not.  Re-read this
    // session: POPCNT SNB..SKL 3, ICL/ADL-P/ADL-E/ARL 0, all Zen 0; TZCNT
    // HSW/BDW 3, SKL+ 0, Zen 0.
    // ------------------------------------------------------------------
    /// POPCNT reads its destination: SNB, IVB, HSW, BDW, SKL, SKX, CLX.
    /// Fixed on ICL/TGL/RKL/ADL-P/ADL-E and on every Zen.
    pub popcnt_false_dep: bool,
    /// LZCNT/TZCNT read their destination: SNB, IVB, HSW, BDW.  Fixed on
    /// SKL and later (measured 0 on SKL/SKX/CLX/ICL/ADL) and on every Zen.
    pub lzcnt_tzcnt_false_dep: bool,

    // ------------------------------------------------------------------
    // Shifts by CL.  `[uops.info]` SHL_R64_CL port usage / rTP, re-read
    // this session: SNB 3*p05 rTP 1.5; **IVB 2*p05 rTP 1.0**; HSW/BDW/SKL/
    // SKX/CLX 3*p06 rTP 1.5; ICL/TGL/RKL/ADL-P/MTL-P/EMR 2*p06 rTP 1.0;
    // ARL-P rTP 0.33; ADL-E rTP 0.54, ARL-E 0.47, Zen2/3/4 0.50, Zen5
    // 0.33 (all 1 µop).  SHLX_R64_R64_R64 is 1 µop with rTP 0.5 (Intel
    // p06) / 0.25–0.33 (Zen) everywhere it exists.
    // ------------------------------------------------------------------
    pub shift_cl_uops: u8,

    // ------------------------------------------------------------------
    // LEA.  `[uops.info]` LEA_B_I_D8_R64 (base+index+disp8), re-read this
    // session: SNB..CLX p1 only, latency 3, rTP 1.0; ICL/TGL/RKL p0156
    // latency 1 rTP 0.25; ADL-P/EMR/MTL-P p0156B latency 1 rTP 0.20;
    // ARL-P latency 1 rTP 0.17; Zen+/Zen2 latency 2 rTP 0.5 (1 µop);
    // Zen3/4/5 latency 2 (base) / 1 (index), rTP 0.5, **2 µops**.
    // ------------------------------------------------------------------
    /// Latency of a 3-component LEA (base + index + displacement).
    pub lea3_latency: u8,
    /// Reciprocal throughput ×100 of the same LEA.
    pub lea3_rtp_x100: u16,
    /// µops of the same LEA (2 on Zen3+, 1 elsewhere).
    pub lea3_uops: u8,

    // ------------------------------------------------------------------
    // CMOV.  `[uops.info]` CMOVB_R64_R64: 2 µops (p015+p05 / p0156+p06),
    // lat 2 on SNB/IVB/HSW; 1 µop lat 1 from BDW on and on every Zen.
    // ADL-E / ARL-E measure latency 2 (operands 2/3 → 1) like the 2-µop
    // SNB form; port usage is not published for the E-cores, so only the
    // latency is recorded for them.
    // ------------------------------------------------------------------
    pub cmov_uops: u8,
    pub cmov_latency: u8,

    // ------------------------------------------------------------------
    // Integer multiply / divide.  `[uops.info]` IMUL_R64_R64 latency 3 on
    // every P-core and Zen (5 on ADL-E/MTL-E, 4 on ARL-E; ARL-P 3–4).
    // DIV_R64 re-read this session — latency is a *range* (dividend
    // magnitude dependent) and the reciprocal throughput is "computed
    // from port usage": SNB 30–91 / 11.0, IVB 31–91 / 11.0, HSW 31–94 /
    // 8.0, BDW 30–92 / 8.0, SKL/SKX/CLX 35–90 / 8.25, ICL/TGL/RKL 14–18 /
    // 3.0, ADL-P/MTL-P/EMR 14–18 / 3.0, ARL-P 16–19, ADL-E 12–43, ARL-E
    // 12–32, Zen+/Zen2 8–41, Zen3/Zen4 9–18, Zen5 10–18.  Zen throughput
    // from `[Agner]` (Zen2 13–44, Zen3/4 7–12).  The prior version of this
    // file listed the *latency* column as throughput for the pre-ICL
    // rows; fixed.
    // ------------------------------------------------------------------
    pub imul64_latency: u8,
    /// Best-case dependent latency of DIV r64 (small dividend).
    pub div64_latency_min: u8,
    /// Worst-case dependent latency of DIV r64 (full 128-bit dividend).
    pub div64_latency: u8,
    /// Reciprocal throughput of DIV r64 ×100.
    pub div64_rtp_x100: u16,
    /// `[uops.info]` PMULLD_XMM_XMM, re-read this session: SNB/IVB 1 µop
    /// lat 5 rTP 1; HSW/BDW 2*p0 lat 10 rTP 2; SKL..ADL-P 2*p01 lat 10
    /// rTP 1; ADL-E lat 4; ARL-P lat 5–6; Zen+/Zen2 lat 4 rTP 1; Zen3/4/5
    /// lat 3 rTP 0.5 (1 µop).  Drives the vectoriser's cost for 32-bit
    /// lane multiplies.
    pub pmulld_uops: u8,
    pub pmulld_latency: u8,

    // ------------------------------------------------------------------
    // Floating point, all `[uops.info]` re-read this session.
    // VFMADD231PS ymm: HSW/BDW 5, SKL..ADL-P 4, ARL-P 4, Zen+/Zen2 5,
    // Zen3/4/5 4, ADL-E 6, ARL-E 4; not present on SNB/IVB.
    // VADDPS ymm: SNB..BDW 3 (p1), SKL..RKL 4 (p01, on the FMA unit),
    // **ADL-P/MTL-P/EMR/ARL-P 2** (dedicated adders on p15), ADL-E 3,
    // Zen+..Zen4 3, Zen5 2.
    // VMULPD ymm: SNB/IVB 5 (p0), HSW 5, BDW 3, SKL..ADL-P 4, MTL-P/ARL-P
    // 3, Zen+ 4, Zen2+ 3.
    // 256-bit FMA-capable pipes: 2 on every listed core except Zen1
    // (2×128 halves → counted as 1).
    // ------------------------------------------------------------------
    pub fma_latency: u8,
    pub fadd_latency: u8,
    pub fmul_latency: u8,
    /// Physical FMA-capable SIMD pipes.  Each pipe is `simd_datapath_bits`
    /// wide, so a 256-bit op on a 128-bit machine (Zen1, Gracemont) holds a
    /// pipe for two cycles — see [`X86Tune::simd_pipes_for`].
    pub fma_pipes: u8,
    /// Physical packed-FP *add* pipes.  `[uops.info]` VADDPS ymm port
    /// usage: SNB/IVB/HSW/BDW **p1 only** (one adder); SKL..RKL p01 (adds
    /// run on both FMA units); ADL-P/RPL/SPR/ARL p15 (two dedicated
    /// adders); Gracemont two 128-bit FP pipes; every Zen FP2+FP3.  The
    /// prior version of this file assumed two adders on every row, which
    /// over-provisioned Haswell/Broadwell FADD reductions by 2×.
    pub fadd_pipes: u8,
    /// Vector integer ALU pipes for VPADDD-class ops.  `[uops.info]`
    /// VPADDD ymm: HSW..ADL-P/ARL-P p015 (3); SNB/IVB p15 on 128-bit only
    /// (no AVX2 → 2); Gracemont two 128-bit pipes; Zen1..Zen5 FP0..FP3
    /// (4, rTP 0.25).
    pub vec_int_alu_pipes: u8,
    /// Native SIMD execution width in bits.  `[Agner microarchitecture]`
    /// / `[AMD SOG]`: SNB..ADL/RPL/ARL client 256 (AVX-512 fused off on
    /// ADL/RPL), SKX/ICL/SPR 512, Gracemont 128 (two 128-bit FP pipes;
    /// 256-bit ops are double-pumped), Zen1 128, Zen2/3/4 256 (Zen4
    /// double-pumps AVX-512), Zen5 desktop 512.  LLVM encodes the same
    /// fact as `TuningPrefer128Bit`/`Prefer256Bit`, GCC as
    /// `-mprefer-vector-width`; here it is data the decisions derive from
    /// ([`X86Tune::prefer_vector_bits`], [`X86Tune::simd_pipes_for`]).
    pub simd_datapath_bits: u16,
    /// Load pipes that can feed SIMD registers per cycle, and the width of
    /// each.  `[Intel ORM]`/`[Agner]`: SNB/IVB 2 × 128 (a 256-bit load
    /// occupies both ports); HSW/BDW/SKL 2 × 256; SKX/ICL 2 × 512; GLC/RPC
    /// (ADL/RPL) 3 × 256 (three load AGUs p2/p3/p11); SPR 3 × 256; LNC
    /// 3 × 256; Gracemont 2 × 128.  `[AMD SOG]`: Zen1 2 × 128; Zen2 2 ×
    /// 256; Zen3/Zen4 2 × 256 vector loads per cycle (the third AGU is
    /// scalar-only); Zen5 2 × 512.
    pub vec_load_ports: u8,
    pub vec_load_port_bits: u16,

    // ------------------------------------------------------------------
    // Memory.  `[cpuid]` ERMS (Enhanced REP MOVSB): IVB+ Intel, all Zen.
    // FSRM (Fast Short REP MOV): ICL+ Intel, Zen4+.  `[Agner]` SNB/IVB
    // execute a 32-byte load as two 16-byte halves; the 256-bit load path
    // is full width from HSW on.
    // ------------------------------------------------------------------
    pub erms: bool,
    pub fsrm: bool,
    pub avx256_unaligned_split: bool,
    /// Smallest copy for which `rep movsb` beats a vector loop, in bytes;
    /// 0 = never use `rep movsb` (no ERMS).  `[glibc]` `dl-cacheinfo.h`:
    /// 2112 on FSRM parts, `4096 × (vector_bytes / 16)` = 8192 for 32-byte
    /// vectors on ERMS-only parts.  glibc switches its own memmove to
    /// `rep movsb` at exactly this size on the same hardware.
    pub rep_movsb_threshold: u16,
    /// Smallest fill for which `rep stosb` beats a vector store loop;
    /// 0 = never.  `[glibc]` `__x86_rep_stosb_threshold` defaults to 2048
    /// on every ERMS part (there is no FSRM distinction for stores).
    pub rep_stosb_threshold: u16,
    /// L1D load-to-use latency (integer, simple addressing) `[Agner]`:
    /// SNB..SKX 4, ICL+ P-cores 5, LNC 4 (L0), Gracemont 3, Zen1–5 4.
    pub load_latency: u8,
    pub cache: CacheModel,
    /// E-core column of hybrid rows; `None` for homogeneous parts.
    pub ecore: Option<ECoreTune>,
}

/// Loop-carried operation class of a vectorised reduction, for
/// [`X86Tune::reduction_interleave`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReductionOp {
    /// `acc = fma(a, b, acc)` — FMA latency, FMA pipes (or add units where
    /// FMA is absent).
    Fma,
    /// `acc = acc + x` in floating point.
    FAdd,
    /// `acc = acc + x` on integer lanes (latency 1).
    IntAdd,
    /// `acc = max(acc, x)` on integer lanes (latency 1, two pipes).
    IntMax,
}

/// Shape of one vectorised reduction loop body (one original iteration).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReductionShape {
    pub op: ReductionOp,
    /// Vector register width of the accumulator (128 or 256).
    pub vector_bits: u16,
    /// Accumulator phis in the loop (each performs one `op` per iteration).
    pub accumulators: u32,
    /// Vector loads per iteration.
    pub loads_per_iter: u32,
    /// Vector µops per iteration including the loads and the accumulator
    /// ops, excluding loop control (modelled as 2: IV add + fused cmp/jcc).
    pub uops_per_iter: u32,
}

/// How a constant-size block copy / fill should be lowered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CopyStrategy {
    /// Straight-line vector/scalar moves, no loop.
    InlineUnrolled,
    /// Counted vector loop (64 B per iteration).
    InlineLoop,
    /// `rep movsb` / `rep stosb` (ERMS/FSRM microcode).
    RepMovsb,
    /// Call the C library: above the non-temporal threshold glibc's
    /// implementation (NT stores, 4-way unrolled, prefetching) beats any
    /// inline sequence, and it does not pollute the cache.
    LibCall,
}

/// One step of a synthesised multiply-by-constant sequence
/// ([`X86Tune::mul_const_plan`]).  `acc` is the destination register, `src`
/// the (unmodified) multiplicand.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MulStep {
    /// `acc = src * scale` for scale ∈ {2,4,8}: `lea (,src,scale), acc`
    /// (index-only LEA, 2-component → 1 cycle everywhere).  Always the
    /// first step when present.
    LeaScale(u8),
    /// `acc = acc + acc*scale` for scale ∈ {2,4,8} (×3 / ×5 / ×9):
    /// `lea (acc,acc,scale), acc` — 2-component LEA, 1 cycle everywhere.
    LeaMul(u8),
    /// `acc <<= k`.
    Shl(u8),
    /// `acc += src` (×(2^k + 1) shapes that exceed LEA's ×9).
    AddSrc,
    /// `acc -= src` (×(2^k − 1)).
    SubSrc,
    /// `acc = -acc`.
    Neg,
}

/// `[uops.info]` ADL-E: LEA_B_I_D8 lat 2 rTP 1.0; SHL r,CL rTP 0.54; IMUL
/// r64 lat 5; DIV r64 lat 12–43; VFMADD231PS ymm lat 6.  `[Agner]` L1D 3.
const GRACEMONT_ADL: ECoreTune = ECoreTune {
    name: "gracemont",
    lea3_latency: 2,
    lea3_rtp_x100: 100,
    shift_cl_uops: 1,
    imul64_latency: 5,
    div64_latency: 43,
    div64_rtp_x100: 1500,
    fma_latency: 6,
    load_latency: 3,
    cluster_l2_kib: 2048,
};

const GRACEMONT_RPL: ECoreTune = ECoreTune {
    cluster_l2_kib: 4096,
    ..GRACEMONT_ADL
};

/// `[uops.info]` ARL-E (Skymont): LEA_B_I_D8 latency 2 rTP 0.29; SHL r,CL
/// rTP 0.47; IMUL r64 lat 4; DIV r64 12–32; VFMADD231PS ymm 4; `[Intel
/// ARK]` 4 MiB per 4-core cluster.
const SKYMONT_ARL: ECoreTune = ECoreTune {
    name: "skymont",
    lea3_latency: 2,
    lea3_rtp_x100: 29,
    shift_cl_uops: 1,
    imul64_latency: 4,
    div64_latency: 32,
    div64_rtp_x100: 1200,
    fma_latency: 4,
    load_latency: 4,
    cluster_l2_kib: 4096,
};

#[allow(clippy::too_many_arguments)]
const fn cache(
    l1d_kib: u32,
    l1d_ways: u8,
    l1d_lat: u8,
    l2_kib: u32,
    l2_ways: u8,
    l2_lat: u8,
    l3_kib: u32,
    l3_ways: u8,
    l3_lat: u8,
    dram_cycles: u16,
) -> CacheModel {
    CacheModel {
        line_bytes: 64,
        l1d: CacheLevel { kib: l1d_kib, ways: l1d_ways, latency: l1d_lat },
        l2: CacheLevel { kib: l2_kib, ways: l2_ways, latency: l2_lat },
        l3: CacheLevel { kib: l3_kib, ways: l3_ways, latency: l3_lat },
        dram_latency_cycles: dram_cycles,
    }
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
            "alderlake" => AlderLake,
            "raptorlake" | "raptor-lake" | "meteorlake" => RaptorLake,
            "gracemont" | "sierraforest" | "grandridge" | "clearwaterforest" | "alderlake-n" => {
                Gracemont
            }
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
        let (is_intel, is_amd) = Self::vendor_flags(&vendor);
        let eax = leaf1.eax;
        let family = ((eax >> 8) & 0xF) + ((eax >> 20) & 0xFF);
        let model = ((eax >> 4) & 0xF) | ((eax >> 12) & 0xF0);
        Self::from_signature(is_intel, is_amd, family, model)
    }

    #[cfg(target_arch = "x86_64")]
    fn vendor_flags(vendor: &std::arch::x86_64::CpuidResult) -> (bool, bool) {
        let vendor_bytes = [
            vendor.ebx.to_le_bytes(),
            vendor.edx.to_le_bytes(),
            vendor.ecx.to_le_bytes(),
        ]
        .concat();
        (
            &vendor_bytes[..] == b"GenuineIntel",
            &vendor_bytes[..] == b"AuthenticAMD",
        )
    }

    /// Read the host's deterministic cache parameters (Intel CPUID leaf 4,
    /// AMD leaf 0x8000001D — identical layout) and overlay them on `base`.
    /// Latencies are kept from the row; only sizes, ways and line size
    /// come from the hardware.  Returns `base` unchanged when the leaf is
    /// unavailable (e.g. under a hypervisor that masks it).
    #[cfg(target_arch = "x86_64")]
    pub fn detect_host_cache(base: CacheModel) -> CacheModel {
        use std::arch::x86_64::{__cpuid, __cpuid_count};
        let vendor = __cpuid(0);
        let (is_intel, is_amd) = Self::vendor_flags(&vendor);
        let leaf = if is_amd {
            if __cpuid(0x8000_0000).eax < 0x8000_001D {
                return base;
            }
            0x8000_001Du32
        } else if is_intel && vendor.eax >= 4 {
            4u32
        } else {
            return base;
        };
        let mut out = base;
        let mut any = false;
        for sub in 0..32u32 {
            let r = __cpuid_count(leaf, sub);
            let ctype = r.eax & 0x1F; // 0 = no more caches, 1 = data, 2 = insn, 3 = unified
            if ctype == 0 {
                break;
            }
            if ctype == 2 {
                continue;
            }
            let level = (r.eax >> 5) & 0x7;
            let ways = ((r.ebx >> 22) & 0x3FF) + 1;
            let partitions = ((r.ebx >> 12) & 0x3FF) + 1;
            let line = (r.ebx & 0xFFF) + 1;
            let sets = r.ecx.wrapping_add(1);
            let bytes = (ways as u64) * (partitions as u64) * (line as u64) * (sets as u64);
            if bytes == 0 || line == 0 || line > 255 {
                continue;
            }
            let kib = (bytes / 1024) as u32;
            let ways8 = ways.min(255) as u8;
            let slot = match (level, ctype) {
                (1, 1) => &mut out.l1d,
                (2, _) => &mut out.l2,
                (3, _) => &mut out.l3,
                _ => continue,
            };
            slot.kib = kib;
            slot.ways = ways8;
            out.line_bytes = line as u8;
            any = true;
        }
        if any {
            out
        } else {
            base
        }
    }

    /// Decode an Intel/AMD family/model signature.  Split out from CPUID so
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
                // ADL-S (0x97), ADL-P (0x9A).
                0x97 | 0x9A => AlderLake,
                // RPL-S (0xB7), RPL-P (0xBA), RPL-S refresh / RPL-HX (0xBF);
                // MTL-M/P (0xAA), MTL-S (0xAC), ARL-U = MTL refresh (0xB5)
                // share the 2 MiB-L2 Redwood/Raptor Cove class.
                0xB7 | 0xBA | 0xBF | 0xAA | 0xAC | 0xB5 => RaptorLake,
                // ADL-N (0xBE), Sierra Forest (0xAF), Grand Ridge (0xB6),
                // Clearwater Forest (0xDD): E-core only.
                0xBE | 0xAF | 0xB6 | 0xDD => Gracemont,
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
                dispatch_width: 4,
                mispredict_penalty: 15,
                rob_entries: 168,
                lsd_uops: 28,
                uop_cache_uops: 1536,
                jcc_erratum: false,
                popcnt_false_dep: true,
                lzcnt_tzcnt_false_dep: true,
                shift_cl_uops: 3,
                lea3_latency: 3,
                lea3_rtp_x100: 100,
                lea3_uops: 1,
                cmov_uops: 2,
                cmov_latency: 2,
                imul64_latency: 3,
                div64_latency_min: 30,
                div64_latency: 91,
                div64_rtp_x100: 1100,
                pmulld_uops: 1,
                pmulld_latency: 5,
                fma_latency: 0,
                fadd_latency: 3,
                fmul_latency: 5,
                fma_pipes: 0,
                // [uops.info] VADDPS ymm SNB: p1 only; 256-bit loads take both
                // 128-bit load ports; no AVX2 → 128-bit integer SIMD on p15.
                fadd_pipes: 1,
                vec_int_alu_pipes: 2,
                simd_datapath_bits: 256,
                vec_load_ports: 2,
                vec_load_port_bits: 128,
                erms,
                fsrm: false,
                avx256_unaligned_split: true,
                rep_movsb_threshold: if erms { 8192 } else { 0 },
                rep_stosb_threshold: if erms { 2048 } else { 0 },
                load_latency: 4,
                // [Intel ARK] i7-2600K/3770K: 32K/8w, 256K/8w, 8M/16w;
                // [Agner] L2 12, L3 ~30 cycles; 3.9 GHz × ~85 ns.
                cache: cache(32, 8, 4, 256, 8, 12, 8192, 16, 30, 330),
                ecore: None,
            }
        }
        match self {
            SandyBridge => snb_like(SandyBridge, "sandybridge", false),
            IvyBridge => X86Tune {
                cpu: IvyBridge,
                name: "ivybridge",
                // [uops.info] SHL_R64_CL on IVB: 2*p05, rTP 1.0 (SNB: 3).
                shift_cl_uops: 2,
                ..snb_like(IvyBridge, "ivybridge", true)
            },
            Haswell => X86Tune {
                cpu: Haswell,
                name: "haswell",
                rob_entries: 192,
                lsd_uops: 56,
                div64_latency_min: 31,
                div64_latency: 94,
                div64_rtp_x100: 800,
                pmulld_uops: 2,
                pmulld_latency: 10,
                fma_latency: 5,
                fma_pipes: 2,
                // [uops.info] HSW: VADDPS still p1 only (the two FMA units do
                // not add until SKL); VPADDD ymm p015; two 256-bit load ports.
                fadd_pipes: 1,
                vec_int_alu_pipes: 3,
                vec_load_port_bits: 256,
                avx256_unaligned_split: false,
                // [Intel ARK] i7-4790K: 32K/8w, 256K/8w, 8M/16w; [Agner] L2
                // 11, L3 ~34; 4.4 GHz × ~80 ns.
                cache: cache(32, 8, 4, 256, 8, 11, 8192, 16, 34, 350),
                ..snb_like(Haswell, "haswell", true)
            },
            Broadwell => X86Tune {
                cpu: Broadwell,
                name: "broadwell",
                cmov_uops: 1,
                cmov_latency: 1,
                div64_latency_min: 30,
                div64_latency: 92,
                fmul_latency: 3,
                ..Haswell.tune()
            },
            Skylake => X86Tune {
                cpu: Skylake,
                name: "skylake",
                mispredict_penalty: 16,
                lzcnt_tzcnt_false_dep: false,
                div64_latency_min: 35,
                div64_latency: 90,
                div64_rtp_x100: 825,
                fma_latency: 4,
                fadd_latency: 4,
                // [uops.info] SKL: VADDPS on p01 (both FMA units).
                fadd_pipes: 2,
                fmul_latency: 4,
                rob_entries: 224,
                lsd_uops: 0,
                jcc_erratum: true,
                // [Intel ARK] i7-9700K: 32K/8w, 256K/4w, 12M/16w; [Agner] L2
                // 12, L3 ~40; 4.9 GHz × ~80 ns.
                cache: cache(32, 8, 4, 256, 4, 12, 12288, 16, 40, 390),
                ..Broadwell.tune()
            },
            SkylakeAvx512 => X86Tune {
                cpu: SkylakeAvx512,
                name: "skylake-avx512",
                // [Intel ARK] Xeon Platinum 8280: 32K/8w, 1M/16w, 1.375M/core
                // (38.5M/11w); [Agner] L2 14, L3 ~70 (mesh); 4.0 GHz × 90 ns.
                cache: cache(32, 8, 4, 1024, 16, 14, 39424, 11, 70, 360),
                // [Intel ORM] SKX: 512-bit FMA on p0+p1 fused (+p5 on the
                // 2-FMA SKUs); two 64-byte loads per cycle.
                simd_datapath_bits: 512,
                vec_load_port_bits: 512,
                ..Skylake.tune()
            },
            IceLake => X86Tune {
                cpu: IceLake,
                name: "icelake-client",
                dispatch_width: 5,
                // [Intel ORM] Sunny Cove: 512-bit FMA on p01 fused, two
                // 64-byte loads per cycle.
                simd_datapath_bits: 512,
                vec_load_port_bits: 512,
                mispredict_penalty: 16,
                popcnt_false_dep: false,
                shift_cl_uops: 2,
                lea3_latency: 1,
                lea3_rtp_x100: 25,
                div64_latency_min: 14,
                div64_latency: 18,
                div64_rtp_x100: 300,
                fsrm: true,
                rep_movsb_threshold: 2112,
                rob_entries: 352,
                lsd_uops: 70,
                uop_cache_uops: 2304,
                jcc_erratum: false,
                load_latency: 5,
                // [Intel ARK] i7-1065G7: 48K/12w, 512K/8w, 8M/16w; [Agner]
                // L1 5, L2 13, L3 ~40; 3.9 GHz × ~90 ns.
                cache: cache(48, 12, 5, 512, 8, 13, 8192, 16, 40, 350),
                ..Skylake.tune()
            },
            AlderLake => X86Tune {
                cpu: AlderLake,
                name: "alderlake",
                dispatch_width: 6,
                // [Intel ORM] Golden Cove: three load AGUs (p2/p3/p11), each
                // 256-bit; AVX-512 fused off → 256-bit datapath; VADDPS on the
                // dedicated p15 adders (fadd_pipes stays 2).
                simd_datapath_bits: 256,
                vec_load_ports: 3,
                vec_load_port_bits: 256,
                mispredict_penalty: 17,
                lea3_rtp_x100: 20,
                // [uops.info] VADDPS ymm on ADL-P: latency 2 (p15 adders).
                fadd_latency: 2,
                rob_entries: 512,
                lsd_uops: 144,
                uop_cache_uops: 4096,
                // [Intel ARK] i9-12900K: 48K/12w, 1.25M/10w, 30M/12w;
                // chipsandcheese GLC: L1 5, L2 ~15, L3 ~55; 5.2 GHz × ~85 ns.
                cache: cache(48, 12, 5, 1280, 10, 15, 30720, 12, 55, 440),
                ecore: Some(GRACEMONT_ADL),
                ..IceLake.tune()
            },
            RaptorLake => X86Tune {
                cpu: RaptorLake,
                name: "raptorlake",
                // [Intel ARK] i7-14700KF: 48K/12w, 2M/16w, 33M/12w; L2
                // latency +1 cycle for the larger array (chipsandcheese RPL
                // review: ~16 vs ~15); 5.6 GHz × ~85 ns.
                cache: cache(48, 12, 5, 2048, 16, 16, 33792, 12, 58, 475),
                ecore: Some(GRACEMONT_RPL),
                ..AlderLake.tune()
            },
            SapphireRapids => X86Tune {
                cpu: SapphireRapids,
                name: "sapphirerapids",
                // [Intel ARK] Xeon 8480+: 48K/12w, 2M/16w, 1.875M/core
                // (105M/15w); mesh L3 ~80 cycles; 3.8 GHz × ~110 ns.
                cache: cache(48, 12, 5, 2048, 16, 16, 107520, 15, 80, 420),
                ecore: None,
                // [Intel ORM] Golden Cove server: AVX-512 enabled, two FMA
                // units; 512-bit loads at 2/cycle (= 3 × 256 / 512 rounded).
                simd_datapath_bits: 512,
                ..AlderLake.tune()
            },
            ArrowLake => X86Tune {
                cpu: ArrowLake,
                name: "arrowlake",
                dispatch_width: 8,
                lea3_rtp_x100: 17,
                // [uops.info] ARL-P: DIV r64 16–19; VMULPD ymm 3; PMULLD 5–6
                // (µop count not published; kept at the GLC value).
                div64_latency_min: 16,
                div64_latency: 19,
                pmulld_latency: 6,
                fmul_latency: 3,
                rob_entries: 576,
                uop_cache_uops: 5250,
                load_latency: 4,
                // [Intel ARK] Core Ultra 9 285K: 48K L0 (4 cy) + 192K L1
                // (9 cy) modelled as the 48K level, 3M/16w L2 ~17, 36M L3
                // ~80 (tile fabric); 5.7 GHz × ~100 ns.
                cache: cache(48, 12, 4, 3072, 16, 17, 36864, 12, 80, 570),
                ecore: Some(SKYMONT_ARL),
                ..AlderLake.tune()
            },
            Gracemont => X86Tune {
                cpu: Gracemont,
                name: "gracemont",
                dispatch_width: 5,
                mispredict_penalty: 13,
                rob_entries: 256,
                lsd_uops: 0,
                uop_cache_uops: 0,
                jcc_erratum: false,
                popcnt_false_dep: false,
                lzcnt_tzcnt_false_dep: false,
                shift_cl_uops: 1,
                lea3_latency: 2,
                lea3_rtp_x100: 100,
                lea3_uops: 1,
                // [uops.info] ADL-E CMOVB: latency 2 (operands 2/3 → 1).
                cmov_uops: 1,
                cmov_latency: 2,
                imul64_latency: 5,
                div64_latency_min: 12,
                div64_latency: 43,
                div64_rtp_x100: 1500,
                pmulld_uops: 1,
                pmulld_latency: 4,
                fma_latency: 6,
                fadd_latency: 3,
                // [Agner] Gracemont: two 128-bit FP/FMA pipes, two 128-bit
                // load pipes; 256-bit ops and loads are double-pumped.
                fadd_pipes: 2,
                vec_int_alu_pipes: 2,
                simd_datapath_bits: 128,
                vec_load_ports: 2,
                vec_load_port_bits: 128,
                fmul_latency: 4,
                fma_pipes: 2,
                erms: true,
                fsrm: true,
                avx256_unaligned_split: false,
                rep_movsb_threshold: 2112,
                rep_stosb_threshold: 2048,
                load_latency: 3,
                // [Intel ARK] Sierra Forest / ADL-N: 32K/8w L1D (3 cy), 4M
                // per 4-core cluster 16w (~17 cy), 6M L3 on ADL-N; 3.4 GHz.
                cache: cache(32, 8, 3, 4096, 16, 17, 6144, 12, 60, 300),
                ecore: None,
            },
            Znver1 => X86Tune {
                cpu: Znver1,
                name: "znver1",
                dispatch_width: 6,
                mispredict_penalty: 18,
                rob_entries: 192,
                lsd_uops: 0,
                uop_cache_uops: 2048,
                jcc_erratum: false,
                popcnt_false_dep: false,
                lzcnt_tzcnt_false_dep: false,
                shift_cl_uops: 1,
                lea3_latency: 2,
                lea3_rtp_x100: 50,
                lea3_uops: 1,
                cmov_uops: 1,
                cmov_latency: 1,
                imul64_latency: 3,
                div64_latency_min: 8,
                div64_latency: 41,
                div64_rtp_x100: 1400,
                pmulld_uops: 1,
                pmulld_latency: 4,
                fma_latency: 5,
                fadd_latency: 3,
                fmul_latency: 4,
                // [AMD SOG] Zen1: FP0/FP1 FMA and FP2/FP3 ADD, all 128-bit;
                // VPADDD on all four pipes; two 128-bit load pipes.  256-bit
                // ops are split, which `simd_datapath_bits: 128` expresses
                // (the previous row folded that into `fma_pipes: 1`).
                fma_pipes: 2,
                fadd_pipes: 2,
                vec_int_alu_pipes: 4,
                simd_datapath_bits: 128,
                vec_load_ports: 2,
                vec_load_port_bits: 128,
                erms: true,
                fsrm: false,
                avx256_unaligned_split: false,
                rep_movsb_threshold: 8192,
                rep_stosb_threshold: 2048,
                load_latency: 4,
                // [AMD SOG 17h] 32K/8w (4 cy), 512K/8w (12 cy), 8M per CCX
                // 16w (~35 cy); 4.0 GHz × ~90 ns.
                cache: cache(32, 8, 4, 512, 8, 12, 8192, 16, 35, 360),
                ecore: None,
            },
            Znver2 => X86Tune {
                cpu: Znver2,
                name: "znver2",
                fma_pipes: 2,
                // [AMD SOG] Zen2: 256-bit FP datapath, two 256-bit loads/cycle.
                simd_datapath_bits: 256,
                vec_load_port_bits: 256,
                fmul_latency: 3,
                rob_entries: 224,
                uop_cache_uops: 4096,
                // [AMD SOG 17h/71h] 16M per CCX (~38 cy); 4.7 GHz × ~80 ns.
                cache: cache(32, 8, 4, 512, 8, 12, 16384, 16, 38, 380),
                ..Znver1.tune()
            },
            Znver3 => X86Tune {
                cpu: Znver3,
                name: "znver3",
                mispredict_penalty: 13,
                lea3_uops: 2,
                div64_latency_min: 9,
                div64_latency: 18,
                div64_rtp_x100: 700,
                pmulld_latency: 3,
                fma_latency: 4,
                rob_entries: 256,
                // [AMD SOG 19h] 32M per CCD 16w (~46 cy); 4.9 GHz × ~80 ns.
                cache: cache(32, 8, 4, 512, 8, 12, 32768, 16, 46, 390),
                ..Znver2.tune()
            },
            Znver4 => X86Tune {
                cpu: Znver4,
                name: "znver4",
                fsrm: true,
                rep_movsb_threshold: 2112,
                rob_entries: 320,
                uop_cache_uops: 6912,
                // [AMD SOG 19h/61h] 1M/8w L2 (14 cy), 32M per CCD (~50 cy);
                // 5.7 GHz × ~75 ns.
                cache: cache(32, 8, 4, 1024, 8, 14, 32768, 16, 50, 430),
                ..Znver3.tune()
            },
            Znver5 => X86Tune {
                cpu: Znver5,
                name: "znver5",
                dispatch_width: 8,
                // [AMD SOG Zen5] full 512-bit FP datapath on the desktop/
                // server dies (mobile Strix Point is 256); two 512-bit loads.
                simd_datapath_bits: 512,
                vec_load_port_bits: 512,
                mispredict_penalty: 14,
                div64_latency_min: 10,
                // [uops.info] VADDPS ymm on Zen 5: latency 2.
                fadd_latency: 2,
                rob_entries: 448,
                uop_cache_uops: 6144,
                // [AMD SOG 1Ah] 48K/12w L1D (4 cy), 1M/16w L2 (14 cy), 32M
                // per CCD (~47 cy); 5.7 GHz × ~75 ns.
                cache: cache(48, 12, 4, 1024, 16, 14, 32768, 16, 47, 430),
                ..Znver4.tune()
            },
            // Generic: the conservative envelope over every row above.  Each
            // field takes the value that is never *harmful*: work-arounds on,
            // latencies at the slow end, structure sizes at the small end,
            // string-op microcode off.  Exception (see module docs): IMUL
            // takes the P-core/Zen value 3, because a pessimistic 5 would
            // make the multiply synthesiser emit 4-op chains that lose on
            // every core except Gracemont.
            Generic => X86Tune {
                cpu: Generic,
                name: "generic",
                dispatch_width: 4,
                mispredict_penalty: 16,
                rob_entries: 168,
                lsd_uops: 0,
                uop_cache_uops: 0,
                jcc_erratum: false,
                popcnt_false_dep: true,
                lzcnt_tzcnt_false_dep: true,
                shift_cl_uops: 3,
                lea3_latency: 3,
                lea3_rtp_x100: 100,
                lea3_uops: 2,
                cmov_uops: 2,
                cmov_latency: 2,
                imul64_latency: 3,
                div64_latency_min: 35,
                div64_latency: 94,
                div64_rtp_x100: 1500,
                pmulld_uops: 2,
                pmulld_latency: 10,
                fma_latency: 6,
                fadd_latency: 4,
                fmul_latency: 5,
                fma_pipes: 1,
                // Envelope: one FP adder (SNB..BDW), 128-bit datapath and
                // 2 × 128-bit loads (SNB/Zen1/Gracemont), 2 vector ALUs.
                fadd_pipes: 1,
                vec_int_alu_pipes: 2,
                simd_datapath_bits: 128,
                vec_load_ports: 2,
                vec_load_port_bits: 128,
                erms: false,
                fsrm: false,
                avx256_unaligned_split: false,
                rep_movsb_threshold: 0,
                rep_stosb_threshold: 0,
                load_latency: 5,
                cache: cache(32, 8, 5, 256, 4, 15, 8192, 16, 80, 570),
                ecore: None,
            },
        }
    }

    /// Every row, for exhaustive tests and `LCCC_DUMP_TUNE=all`.
    pub const ALL: [X86Cpu; 18] = [
        X86Cpu::Generic,
        X86Cpu::SandyBridge,
        X86Cpu::IvyBridge,
        X86Cpu::Haswell,
        X86Cpu::Broadwell,
        X86Cpu::Skylake,
        X86Cpu::SkylakeAvx512,
        X86Cpu::IceLake,
        X86Cpu::AlderLake,
        X86Cpu::RaptorLake,
        X86Cpu::SapphireRapids,
        X86Cpu::ArrowLake,
        X86Cpu::Gracemont,
        X86Cpu::Znver1,
        X86Cpu::Znver2,
        X86Cpu::Znver3,
        X86Cpu::Znver4,
        X86Cpu::Znver5,
    ];
}

/// A synthesised multiply-by-constant sequence: at most four steps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MulPlan {
    steps: [MulStep; 4],
    len: u8,
    /// The plan reads `src` after the first step wrote `acc`, so it is
    /// only valid when the multiplicand and the destination are different
    /// registers.
    pub needs_distinct_src: bool,
}

impl MulPlan {
    const fn new(steps: &[MulStep], needs_distinct_src: bool) -> MulPlan {
        let mut arr = [MulStep::Neg; 4];
        let mut i = 0;
        while i < steps.len() {
            arr[i] = steps[i];
            i += 1;
        }
        MulPlan { steps: arr, len: steps.len() as u8, needs_distinct_src }
    }
    pub fn steps(&self) -> &[MulStep] {
        &self.steps[..self.len as usize]
    }
    pub fn len(&self) -> usize {
        self.len as usize
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    fn push(mut self, s: MulStep) -> MulPlan {
        self.steps[self.len as usize] = s;
        self.len += 1;
        self
    }
}

fn lea_mul_scale(m: i64) -> Option<u8> {
    match m {
        3 => Some(2),
        5 => Some(4),
        9 => Some(8),
        _ => None,
    }
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
    /// everywhere; the legacy form is 2–3 µops on Intel P-cores and 1 on
    /// Zen / Gracemont, so it is only *always* preferable where the legacy
    /// form is multi-µop.
    #[inline]
    pub fn prefer_shlx(&self, bmi2: bool) -> bool {
        bmi2 && self.shift_cl_uops > 1
    }

    /// Select the VEX form when it additionally saves the `mov %r,%rcx`
    /// (three-operand form takes the count from any register).  A pure win
    /// on every core that has BMI2.
    #[inline]
    pub fn shlx_saves_move(&self, bmi2: bool) -> bool {
        bmi2
    }

    /// Effective number of `pipes` available to one `vector_bits`-wide
    /// operation per cycle: a 256-bit op on a 128-bit datapath (Zen1,
    /// Gracemont) holds its pipe for two cycles, so two physical pipes
    /// sustain one such op per cycle.
    #[inline]
    pub fn simd_pipes_for(&self, pipes: u8, vector_bits: u16) -> f64 {
        let dp = self.simd_datapath_bits.max(128) as f64;
        let scale = (dp / vector_bits.max(128) as f64).min(1.0);
        pipes as f64 * scale
    }

    /// Sustained `vector_bits`-wide loads per cycle from L1D.
    #[inline]
    pub fn vector_loads_per_cycle(&self, vector_bits: u16) -> f64 {
        let w = self.vec_load_port_bits.max(128) as f64;
        let scale = (w / vector_bits.max(128) as f64).min(1.0);
        self.vec_load_ports.max(1) as f64 * scale
    }

    /// Vector width (bits) that runs at full rate on this part — the
    /// number GCC exposes as `-mprefer-vector-width` and LLVM as
    /// `TuningPrefer128Bit/256Bit`.  Hybrid rows follow the P-core: the
    /// E-core double-pumps but does not *penalise* 256-bit code beyond
    /// the halved rate, and the hot code is scheduled on the P-core.
    #[inline]
    pub fn prefer_vector_bits(&self) -> u16 {
        self.simd_datapath_bits.clamp(128, 512)
    }

    /// Independent accumulators needed to saturate the FMA pipes on a
    /// 256-bit reduction (Little's law: latency × effective pipes).
    /// 0 = no FMA.
    #[inline]
    pub fn fma_reduction_accumulators(&self) -> u8 {
        (self.fma_latency as f64 * self.simd_pipes_for(self.fma_pipes, 256).max(1.0)).ceil()
            as u8
    }

    /// Independent accumulators for a 256-bit FADD reduction (latency ×
    /// effective adder pipes; one adder on SNB..BDW, two elsewhere).
    #[inline]
    pub fn fadd_reduction_accumulators(&self) -> u8 {
        (self.fadd_latency as f64 * self.simd_pipes_for(self.fadd_pipes, 256).max(1.0)).ceil()
            as u8
    }

    /// Interleave factor (independent accumulator chains per reduction
    /// phi) for a vectorised reduction loop, from a roofline of the loop
    /// body on this row.
    ///
    /// With `k` chains per accumulator one *group* iteration (k original
    /// iterations) takes
    ///
    /// ```text
    /// T(k) = max( latency,                          // each chain: 1 op / iteration
    ///             (k·uops + 2) / dispatch_width,    // front end (+ IV add, fused cmp/jcc)
    ///             k·loads / loads_per_cycle,         // L1D load ports
    ///             k·accs  / eff_pipes )              // execution pipes of the op
    /// ```
    ///
    /// cycles, i.e. `T(k)/k` cycles per original iteration, which is
    /// non-increasing in `k`.  The smallest `k ∈ {1,2,4,8}` within 5 % of
    /// the cost at the register cap is chosen; the cap keeps
    /// `k × accs ≤ 8` so eight of the sixteen SIMD registers stay free for
    /// loads, broadcasts and the address temporaries the allocator needs.
    ///
    /// Two floors, both LLVM parity (`X86TTIImpl::getMaxInterleaveFactor`
    /// returns 4 with AVX, 2 without): a 256-bit loop never interleaves
    /// below 4 and a 128-bit loop never below 2 when the cap allows.  The
    /// nominal `dispatch_width` in `T(k)` assumes ideal µop delivery; the
    /// floor covers the sustained-below-nominal reality (DSB/LSD bandwidth,
    /// taken-branch bubbles) that the roofline does not see, exactly the
    /// loop-overhead amortisation LLVM's constant buys.
    ///
    /// Worked examples (see the unit tests): FMA dot product, 2 loads/op,
    /// 256-bit — Skylake (lat 4, 2 × 256-bit loads, 4-wide) → 4: the loop
    /// is load-bound at 1 FMA/cycle and 4 chains already reach it; Raptor
    /// Lake (3 loads/cycle, 6-wide) → 8: 4 chains leave the third load
    /// port idle (1.0 vs 1.5 FMA/cycle).  Integer add reductions stay at
    /// 4 everywhere (latency 1).
    pub fn reduction_interleave(&self, shape: ReductionShape) -> u32 {
        let bits = shape.vector_bits.max(128);
        let accs = shape.accumulators.max(1);
        let (latency, pipes) = match shape.op {
            ReductionOp::Fma if self.fma_latency > 0 => (self.fma_latency, self.fma_pipes),
            // No FMA on this row (SNB/IVB): the reduction is mul + add on
            // separate units; the add is the loop-carried op.
            ReductionOp::Fma => (self.fadd_latency, self.fadd_pipes),
            ReductionOp::FAdd => (self.fadd_latency, self.fadd_pipes),
            ReductionOp::IntAdd => (1, self.vec_int_alu_pipes),
            // PMAXSD runs on p01 (SKL+) / p1 (HSW) / two Zen pipes.
            ReductionOp::IntMax => (1, self.vec_int_alu_pipes.min(2)),
        };
        let latency = latency.max(1) as f64;
        let eff_pipes = self.simd_pipes_for(pipes.max(1), bits).max(0.5);
        let loads_per_cycle = self.vector_loads_per_cycle(bits).max(0.5);
        let width = self.issue_width() as f64;
        let uops = shape.uops_per_iter.max(accs) as f64;
        let loads = shape.loads_per_iter as f64;
        let accs_f = accs as f64;
        let t = |k: u32| -> f64 {
            let k = k as f64;
            latency
                .max((k * uops + 2.0) / width)
                .max(k * loads / loads_per_cycle)
                .max(k * accs_f / eff_pipes)
        };
        const REG_BUDGET: u32 = 8;
        let cap = (REG_BUDGET / accs).max(1);
        let cap_pow2 = if cap >= 8 { 8 } else if cap >= 4 { 4 } else if cap >= 2 { 2 } else { 1 };
        let best = t(cap_pow2) / cap_pow2 as f64;
        let mut chosen = cap_pow2;
        for k in [1u32, 2, 4] {
            if k > cap_pow2 {
                break;
            }
            if t(k) / k as f64 <= best * 1.05 {
                chosen = k;
                break;
            }
        }
        let floor = if bits >= 256 { 4 } else { 2 };
        chosen.max(floor.min(cap_pow2))
    }

    /// Instruction budget per arm for branch → select (if-)conversion when
    /// no profile is available.  A converted branch pays both arms plus the
    /// select unconditionally; an unconverted branch pays
    /// `P(mispredict) × penalty`.  With the coin-flip prior `P = 1/2` the
    /// break-even arm length is `penalty / 2`.  Clamped to [4, 12] so a
    /// single row can never disable the transform or blow up code size.
    /// `Generic` (penalty 16) reproduces the historical constant 8.
    #[inline]
    pub fn if_convert_arm_budget(&self) -> usize {
        (self.mispredict_penalty as usize / 2).clamp(4, 12)
    }

    /// Throughput bound used by cycle estimates (`ops / width`).
    #[inline]
    pub fn issue_width(&self) -> usize {
        self.dispatch_width.max(1) as usize
    }

    /// A 3-component LEA on the loop-carried critical path costs more than
    /// `lea + add` on this part (SNB..CLX: 3 vs 2 cycles; Zen3+: 2 µops;
    /// Gracemont/hybrid E-core: 2 cycles at throughput 1.0).  ICL/ADL-P
    /// alone would say `false`; the hybrid rows say `true` because the
    /// P-core is indifferent (1 cycle either way at rTP ≤ 0.25) while the
    /// E-core is not.
    #[inline]
    pub fn avoid_lea3_on_critical_path(&self) -> bool {
        self.lea3_latency > 1
            || self.lea3_uops > 1
            || matches!(self.ecore, Some(e) if e.lea3_latency > 1 && e.lea3_rtp_x100 >= 100)
    }

    /// Maximum number of 1-cycle ALU/LEA steps a synthesised
    /// multiply-by-constant may use instead of one `imul $k`.  A chain of
    /// `n` single-cycle steps has latency `n`; it beats IMUL on the
    /// critical path only when `n < imul_latency`, and it costs `n − 1`
    /// extra µops, so the budget is `imul_latency − 1`: 2 on every P-core
    /// and Zen (IMUL 3), 4 on Gracemont (IMUL 5).  This reproduces GCC's
    /// and LLVM's 2-instruction decompositions (`lea+add`, `shl+sub`) on
    /// generic tuning and goes further only where the divider-class IMUL
    /// of the E-core justifies it.  Hybrid rows follow the P-core (the
    /// hot code runs there) — Gracemont's IMUL is *not* pathological, so
    /// the E-core rule does not apply.
    #[inline]
    pub fn mul_const_op_budget(&self) -> usize {
        (self.imul64_latency as usize).saturating_sub(1).clamp(1, 4)
    }

    /// Synthesise `acc = src * k` from LEA/SHL/ADD/SUB/NEG steps when that
    /// is shorter in latency than `imul $k` on this core; `None` means
    /// "use IMUL".  `k` must not be 0 or 1.  The plan's
    /// `needs_distinct_src` tells the caller whether the multiplicand
    /// must survive in a register other than the destination.
    pub fn mul_const_plan(&self, k: i64) -> Option<MulPlan> {
        if k == 0 || k == 1 {
            return None;
        }
        let budget = self.mul_const_op_budget();
        let neg = k < 0;
        let m = k.unsigned_abs();
        if m > i32::MAX as u64 + 1 {
            return None;
        }
        let m = m as i64;
        let extra = usize::from(neg);
        let fits = |p: &MulPlan| p.len() + extra <= budget;

        let mut best: Option<MulPlan> = None;
        let mut consider = |p: MulPlan| {
            if !fits(&p) {
                return;
            }
            // Shortest wins; on a tie prefer the in-place plan (no
            // register constraint) over the one that needs a distinct
            // source.
            let better = match &best {
                None => true,
                Some(b) => {
                    p.len() < b.len() || (p.len() == b.len() && b.needs_distinct_src && !p.needs_distinct_src)
                }
            };
            if better {
                best = Some(p);
            }
        };

        // Chains: (3|5|9)^a × 2^s, a ≤ 2, applied in place.
        let mut base = m;
        let mut shift = 0u8;
        while base & 1 == 0 && base > 1 {
            base >>= 1;
            shift += 1;
        }
        let tail = |p: MulPlan, shift: u8| if shift > 0 { p.push(MulStep::Shl(shift)) } else { p };
        if base == 1 {
            // Pure power of two: `shl` (or `add acc,acc` — same cost).
            consider(MulPlan::new(&[MulStep::Shl(shift)], false));
        }
        if let Some(sc) = lea_mul_scale(base) {
            consider(tail(MulPlan::new(&[MulStep::LeaMul(sc)], false), shift));
        }
        for m1 in [3i64, 5, 9] {
            if base % m1 == 0 {
                if let Some(sc2) = lea_mul_scale(base / m1) {
                    let sc1 = lea_mul_scale(m1).unwrap();
                    consider(tail(
                        MulPlan::new(&[MulStep::LeaMul(sc1), MulStep::LeaMul(sc2)], false),
                        shift,
                    ));
                }
            }
        }
        // 2^s ± 1 (and ×2^t of those): need the source in a second
        // register.  ×(2^s+1) with s ≤ 3 is a LeaMul already.
        if base >= 3 {
            let plus = (base - 1) as u64;
            let minus = (base + 1) as u64;
            if plus.is_power_of_two() && plus > 8 {
                let s = plus.trailing_zeros() as u8;
                consider(tail(MulPlan::new(&[MulStep::Shl(s), MulStep::AddSrc], true), shift));
            }
            if minus.is_power_of_two() && minus >= 4 {
                let s = minus.trailing_zeros() as u8;
                let first = if minus <= 8 { MulStep::LeaScale(minus as u8) } else { MulStep::Shl(s) };
                consider(tail(MulPlan::new(&[first, MulStep::SubSrc], true), shift));
            }
        }
        let mut p = best?;
        if neg {
            p = p.push(MulStep::Neg);
        }
        // Never longer in latency than IMUL itself.
        if p.len() >= self.imul64_latency as usize {
            return None;
        }
        Some(p)
    }

    /// Lowering for a constant-size block copy of `size` bytes (a struct
    /// assignment, which can only be emitted inline) with
    /// `vector_bytes`-wide moves available (16 or 32).
    ///
    /// * up to 8 vector moves (GCC's `move_ratio`, Clang's
    ///   `MaxStoresPerMemcpy`): straight-line;
    /// * above `rep_movsb_threshold` on ERMS/FSRM parts: `rep movsb`,
    ///   which is what glibc's memmove does for the same size on the same
    ///   hardware `[glibc]`, in three instructions instead of a loop;
    /// * otherwise the counted vector loop.
    #[inline]
    pub fn memcpy_strategy(&self, size: usize, vector_bytes: usize) -> CopyStrategy {
        let vb = vector_bytes.max(1);
        if size <= 8 * vb {
            return CopyStrategy::InlineUnrolled;
        }
        let threshold = self.rep_movsb_threshold_for(vb);
        if threshold != 0 && size >= threshold {
            return CopyStrategy::RepMovsb;
        }
        CopyStrategy::InlineLoop
    }

    /// `rep movsb` crossover for an inline loop that moves `vector_bytes`
    /// per instruction.  glibc `dl-cacheinfo.h` derives the threshold from
    /// the vector width its own memmove uses, because the competitor of
    /// `rep movsb` is *that* loop: `2048 × (16/16)` with SSE2 vectors,
    /// `4096 × (32/16) = 8192` with AVX, `4096 × (64/16) = 16384` with
    /// AVX-512, and a flat 2112 on FSRM parts where the microcode handles
    /// short copies well.  The row field `rep_movsb_threshold` records the
    /// 32-byte figure; a 16-byte inline loop (baseline `-march`) moves half
    /// the bytes per instruction, so its crossover is glibc's SSE2 number.
    /// 0 = never (no ERMS).
    #[inline]
    pub fn rep_movsb_threshold_for(&self, vector_bytes: usize) -> usize {
        if !self.erms || self.rep_movsb_threshold == 0 {
            return 0;
        }
        if self.fsrm {
            return self.rep_movsb_threshold as usize;
        }
        match vector_bytes {
            0..=16 => 2048,
            17..=32 => self.rep_movsb_threshold as usize,
            _ => 16384,
        }
    }

    /// Byte count above which a constant-size `memcpy`/`memset` *call* is
    /// left to the C library.  ERMS rows: glibc's non-temporal threshold,
    /// `L3 / 4` since glibc 2.38 (`dl-cacheinfo.h`, "non_temporal_threshold
    /// … roughly sizeof_L3 / 4") — above it glibc streams with NT stores,
    /// which no inline sequence can match without polluting the cache.
    /// Non-ERMS rows: 8192, GCC's own loop→libcall boundary for generic
    /// x86-64 (`ix86_tune_cost->memcpy`).
    #[inline]
    pub fn libcall_above_bytes(&self) -> usize {
        if self.erms {
            (self.cache.l3.kib as usize * 1024) / 4
        } else {
            8192
        }
    }

    /// Lowering for a constant-size `memcpy(dst, src, size)` *call*: as
    /// [`Self::memcpy_strategy`], but huge copies stay a libcall.
    #[inline]
    pub fn memcpy_call_strategy(&self, size: usize, vector_bytes: usize) -> CopyStrategy {
        if size > self.libcall_above_bytes() {
            return CopyStrategy::LibCall;
        }
        self.memcpy_strategy(size, vector_bytes)
    }

    /// Lowering for a constant-size, constant-value `memset(dst, c, size)`
    /// call.  Straight-line stores up to 8 vectors (GCC `move_ratio` /
    /// Clang `MaxStoresPerMemset`), `rep stosb` from glibc's
    /// `__x86_rep_stosb_threshold` (2048) on ERMS parts, a vector store
    /// loop in between, and the library above the non-temporal bound.
    #[inline]
    pub fn memset_strategy(&self, size: usize, vector_bytes: usize) -> CopyStrategy {
        let vb = vector_bytes.max(1);
        if size <= 8 * vb {
            return CopyStrategy::InlineUnrolled;
        }
        if size > self.libcall_above_bytes() {
            return CopyStrategy::LibCall;
        }
        if self.erms && self.rep_stosb_threshold != 0 && size >= self.rep_stosb_threshold as usize
        {
            return CopyStrategy::RepMovsb;
        }
        CopyStrategy::InlineLoop
    }

    /// Width of the vector moves a block copy should use when AVX is
    /// enabled: 16 B on parts whose 256-bit load path is split (SNB/IVB),
    /// 32 B elsewhere.
    #[inline]
    pub fn block_copy_vector_bytes(&self, avx2: bool) -> usize {
        if avx2 && !self.avx256_unaligned_split {
            32
        } else {
            16
        }
    }

    /// Working-set size (bytes) above which a streaming loop should not
    /// expect its data to stay on-core: the private L2.  Used by tiling /
    /// non-temporal heuristics as a scale, not as an exact figure.
    #[inline]
    pub fn on_core_bytes(&self) -> usize {
        self.cache.l2.kib as usize * 1024
    }

    /// Human-readable dump for `LCCC_DUMP_TUNE`.  Key order is stable and
    /// pinned by tests; scripts (`scripts/tune_oracle.py`) parse it.
    pub fn dump(&self) -> String {
        let mut s = String::with_capacity(2048);
        let mut kv = |k: &str, v: String| {
            s.push_str(k);
            s.push('=');
            s.push_str(&v);
            s.push('\n');
        };
        kv("tune", self.name.to_string());
        kv("dispatch_width", self.dispatch_width.to_string());
        kv("mispredict_penalty", self.mispredict_penalty.to_string());
        kv("rob_entries", self.rob_entries.to_string());
        kv("lsd_uops", self.lsd_uops.to_string());
        kv("uop_cache_uops", self.uop_cache_uops.to_string());
        kv("jcc_erratum", self.jcc_erratum.to_string());
        kv("popcnt_false_dep", self.popcnt_false_dep.to_string());
        kv("lzcnt_tzcnt_false_dep", self.lzcnt_tzcnt_false_dep.to_string());
        kv("shift_cl_uops", self.shift_cl_uops.to_string());
        kv("fadd_pipes", self.fadd_pipes.to_string());
        kv("vec_int_alu_pipes", self.vec_int_alu_pipes.to_string());
        kv("simd_datapath_bits", self.simd_datapath_bits.to_string());
        kv("vec_load_ports", self.vec_load_ports.to_string());
        kv("vec_load_port_bits", self.vec_load_port_bits.to_string());
        kv("derived.prefer_vector_bits", self.prefer_vector_bits().to_string());
        kv(
            "derived.reduction_interleave_fma_dot_256",
            self.reduction_interleave(ReductionShape {
                op: ReductionOp::Fma,
                vector_bits: 256,
                accumulators: 1,
                loads_per_iter: 2,
                uops_per_iter: 2,
            })
            .to_string(),
        );
        kv(
            "derived.reduction_interleave_fadd_sum_256",
            self.reduction_interleave(ReductionShape {
                op: ReductionOp::FAdd,
                vector_bits: 256,
                accumulators: 1,
                loads_per_iter: 1,
                uops_per_iter: 1,
            })
            .to_string(),
        );
        kv(
            "derived.reduction_interleave_iadd_sum_256",
            self.reduction_interleave(ReductionShape {
                op: ReductionOp::IntAdd,
                vector_bits: 256,
                accumulators: 1,
                loads_per_iter: 1,
                uops_per_iter: 1,
            })
            .to_string(),
        );
        kv("lea3_latency", self.lea3_latency.to_string());
        kv("lea3_rtp_x100", self.lea3_rtp_x100.to_string());
        kv("lea3_uops", self.lea3_uops.to_string());
        kv("cmov_uops", self.cmov_uops.to_string());
        kv("cmov_latency", self.cmov_latency.to_string());
        kv("imul64_latency", self.imul64_latency.to_string());
        kv("div64_latency_min", self.div64_latency_min.to_string());
        kv("div64_latency", self.div64_latency.to_string());
        kv("div64_rtp_x100", self.div64_rtp_x100.to_string());
        kv("pmulld_uops", self.pmulld_uops.to_string());
        kv("pmulld_latency", self.pmulld_latency.to_string());
        kv("fma_latency", self.fma_latency.to_string());
        kv("fadd_latency", self.fadd_latency.to_string());
        kv("fmul_latency", self.fmul_latency.to_string());
        kv("fma_pipes", self.fma_pipes.to_string());
        kv("erms", self.erms.to_string());
        kv("fsrm", self.fsrm.to_string());
        kv("avx256_unaligned_split", self.avx256_unaligned_split.to_string());
        kv("rep_movsb_threshold", self.rep_movsb_threshold.to_string());
        kv("rep_stosb_threshold", self.rep_stosb_threshold.to_string());
        kv("load_latency", self.load_latency.to_string());
        kv("cache.line_bytes", self.cache.line_bytes.to_string());
        kv("cache.l1d_kib", self.cache.l1d.kib.to_string());
        kv("cache.l1d_ways", self.cache.l1d.ways.to_string());
        kv("cache.l1d_latency", self.cache.l1d.latency.to_string());
        kv("cache.l2_kib", self.cache.l2.kib.to_string());
        kv("cache.l2_ways", self.cache.l2.ways.to_string());
        kv("cache.l2_latency", self.cache.l2.latency.to_string());
        kv("cache.l3_kib", self.cache.l3.kib.to_string());
        kv("cache.l3_ways", self.cache.l3.ways.to_string());
        kv("cache.l3_latency", self.cache.l3.latency.to_string());
        kv("cache.dram_latency_cycles", self.cache.dram_latency_cycles.to_string());
        match self.ecore {
            Some(e) => {
                kv("ecore", e.name.to_string());
                kv("ecore.lea3_latency", e.lea3_latency.to_string());
                kv("ecore.lea3_rtp_x100", e.lea3_rtp_x100.to_string());
                kv("ecore.shift_cl_uops", e.shift_cl_uops.to_string());
                kv("ecore.imul64_latency", e.imul64_latency.to_string());
                kv("ecore.div64_latency", e.div64_latency.to_string());
                kv("ecore.div64_rtp_x100", e.div64_rtp_x100.to_string());
                kv("ecore.fma_latency", e.fma_latency.to_string());
                kv("ecore.load_latency", e.load_latency.to_string());
                kv("ecore.cluster_l2_kib", e.cluster_l2_kib.to_string());
            }
            None => kv("ecore", "none".to_string()),
        }
        kv(
            "derived.fma_reduction_accumulators",
            self.fma_reduction_accumulators().to_string(),
        );
        kv(
            "derived.fadd_reduction_accumulators",
            self.fadd_reduction_accumulators().to_string(),
        );
        kv("derived.if_convert_arm_budget", self.if_convert_arm_budget().to_string());
        kv("derived.issue_width", self.issue_width().to_string());
        kv("derived.prefer_shlx", self.prefer_shlx(true).to_string());
        kv(
            "derived.avoid_lea3_on_critical_path",
            self.avoid_lea3_on_critical_path().to_string(),
        );
        kv("derived.mul_const_op_budget", self.mul_const_op_budget().to_string());
        kv(
            "derived.mul_const_plan_100",
            match self.mul_const_plan(100) {
                Some(p) => format!("{:?}", p.steps()),
                None => "Imul".to_string(),
            },
        );
        kv("derived.block_copy_vector_bytes_avx2", self.block_copy_vector_bytes(true).to_string());
        kv(
            "derived.memcpy_4096_avx2",
            format!("{:?}", self.memcpy_strategy(4096, 32)),
        );
        kv(
            "derived.memset_4096_avx2",
            format!("{:?}", self.memset_strategy(4096, 32)),
        );
        kv("derived.libcall_above_bytes", self.libcall_above_bytes().to_string());
        kv("derived.on_core_bytes", self.on_core_bytes().to_string());
        s
    }
}

impl Default for X86Tune {
    fn default() -> Self {
        X86Tune::GENERIC
    }
}

/// Resolve the row for a compilation: `-mtune` wins over `-march`, `native`
/// goes through CPUID (core identity from leaf 1, cache geometry from leaf
/// 4 / 0x8000001D), unknown names fall back to `Generic`.
pub fn resolve(march: Option<&str>, mtune: Option<&str>) -> X86Tune {
    fn one(name: &str) -> Option<(X86Cpu, bool)> {
        if name == "native" {
            #[cfg(target_arch = "x86_64")]
            {
                return Some((X86Cpu::detect_host(), true));
            }
            #[cfg(not(target_arch = "x86_64"))]
            {
                return Some((X86Cpu::Generic, false));
            }
        }
        X86Cpu::from_name(name).map(|c| (c, false))
    }
    let (cpu, native) = mtune
        .and_then(one)
        .or_else(|| march.and_then(one))
        .unwrap_or((X86Cpu::Generic, false));
    let mut t = cpu.tune();
    if native {
        #[cfg(target_arch = "x86_64")]
        {
            t.cache = X86Cpu::detect_host_cache(t.cache);
        }
    }
    t
}

static ACTIVE: OnceLock<std::sync::RwLock<X86Tune>> = OnceLock::new();

fn active_cell() -> &'static std::sync::RwLock<X86Tune> {
    ACTIVE.get_or_init(|| std::sync::RwLock::new(X86Tune::GENERIC))
}

/// Publish the row for this compilation so target-independent passes
/// (`if_convert`, `reassoc_accum`, …) and the MachInst selector can query
/// it through [`active`].
pub fn set_active(t: X86Tune) {
    *active_cell().write().unwrap_or_else(|e| e.into_inner()) = t;
}

/// The row published by [`set_active`] (`Generic` before the driver runs).
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
        assert!(Skylake.tune().popcnt_false_dep && !Skylake.tune().lzcnt_tzcnt_false_dep);
    }

    #[test]
    fn shift_cl_uops_match_uops_info() {
        use X86Cpu::*;
        for cpu in [SandyBridge, Haswell, Broadwell, Skylake, SkylakeAvx512] {
            assert_eq!(cpu.tune().shift_cl_uops, 3, "{:?}", cpu);
        }
        // IVB is the one pre-Skylake core with a 2-µop SHL r,CL (2*p05).
        for cpu in [IvyBridge, IceLake, AlderLake, RaptorLake, SapphireRapids, ArrowLake] {
            assert_eq!(cpu.tune().shift_cl_uops, 2, "{:?}", cpu);
        }
        for cpu in [Gracemont, Znver1, Znver2, Znver3, Znver4, Znver5] {
            assert_eq!(cpu.tune().shift_cl_uops, 1, "{:?}", cpu);
            assert!(!cpu.tune().prefer_shlx(true));
            assert!(cpu.tune().shlx_saves_move(true));
        }
        for cpu in [Haswell, Skylake, IceLake, AlderLake, RaptorLake, Generic] {
            assert!(cpu.tune().prefer_shlx(true));
            assert!(!cpu.tune().prefer_shlx(false));
        }
    }

    #[test]
    fn lea_and_cmov_and_div_rows() {
        use X86Cpu::*;
        for cpu in [SandyBridge, Haswell, Skylake, SkylakeAvx512] {
            assert_eq!(cpu.tune().lea3_latency, 3);
            assert!(cpu.tune().avoid_lea3_on_critical_path());
        }
        for cpu in [IceLake, AlderLake, RaptorLake, ArrowLake] {
            assert_eq!(cpu.tune().lea3_latency, 1);
        }
        // ICL alone is indifferent; the hybrid rows avoid it for Gracemont.
        assert!(!IceLake.tune().avoid_lea3_on_critical_path());
        assert!(!SapphireRapids.tune().avoid_lea3_on_critical_path());
        assert!(AlderLake.tune().avoid_lea3_on_critical_path());
        assert!(RaptorLake.tune().avoid_lea3_on_critical_path());
        // Skymont's LEA is 2 cycles but pipelined (rTP 0.29): not avoided.
        assert!(!ArrowLake.tune().avoid_lea3_on_critical_path());
        assert!(Gracemont.tune().avoid_lea3_on_critical_path());
        assert!(Znver2.tune().avoid_lea3_on_critical_path());
        assert_eq!(Znver3.tune().lea3_uops, 2);
        for cpu in [SandyBridge, IvyBridge, Haswell] {
            assert_eq!(cpu.tune().cmov_uops, 2);
        }
        for cpu in [Broadwell, Skylake, IceLake, AlderLake, Znver1, Znver5] {
            assert_eq!(cpu.tune().cmov_uops, 1);
            assert_eq!(cpu.tune().cmov_latency, 1);
        }
        assert_eq!(Gracemont.tune().cmov_latency, 2);
        // DIV r64: latency is a range, throughput from port usage.
        for cpu in X86Cpu::ALL {
            let t = cpu.tune();
            assert!(t.div64_latency_min <= t.div64_latency, "{:?}", cpu);
            assert!(t.div64_rtp_x100 >= 300, "{:?}", cpu);
        }
        assert_eq!(SandyBridge.tune().div64_rtp_x100, 1100);
        assert_eq!(Haswell.tune().div64_rtp_x100, 800);
        assert_eq!(Skylake.tune().div64_rtp_x100, 825);
        assert_eq!(IceLake.tune().div64_rtp_x100, 300);
        assert_eq!(RaptorLake.tune().div64_latency, 18);
        assert!(Skylake.tune().div64_latency > IceLake.tune().div64_latency);
        assert!(IceLake.tune().div64_latency_min > Znver3.tune().div64_latency_min);
        assert_eq!(Znver2.tune().div64_rtp_x100, 1400);
        assert_eq!(Znver3.tune().div64_rtp_x100, 700);
        assert_eq!(Gracemont.tune().imul64_latency, 5);
    }

    #[test]
    fn fp_latencies_match_uops_info() {
        use X86Cpu::*;
        // VADDPS ymm: p1 adder 3 (SNB..BDW), FMA unit 4 (SKL..RKL),
        // dedicated adders 2 (GLC/RPC/LNC, Zen 5).
        for cpu in [SandyBridge, Haswell, Broadwell] {
            assert_eq!(cpu.tune().fadd_latency, 3, "{:?}", cpu);
        }
        for cpu in [Skylake, SkylakeAvx512, IceLake] {
            assert_eq!(cpu.tune().fadd_latency, 4, "{:?}", cpu);
        }
        for cpu in [AlderLake, RaptorLake, SapphireRapids, ArrowLake, Znver5] {
            assert_eq!(cpu.tune().fadd_latency, 2, "{:?}", cpu);
        }
        for cpu in [Gracemont, Znver1, Znver2, Znver3, Znver4] {
            assert_eq!(cpu.tune().fadd_latency, 3, "{:?}", cpu);
        }
        // VMULPD ymm.
        assert_eq!(SandyBridge.tune().fmul_latency, 5);
        assert_eq!(Haswell.tune().fmul_latency, 5);
        assert_eq!(Broadwell.tune().fmul_latency, 3);
        assert_eq!(Skylake.tune().fmul_latency, 4);
        assert_eq!(RaptorLake.tune().fmul_latency, 4);
        assert_eq!(ArrowLake.tune().fmul_latency, 3);
        assert_eq!(Znver1.tune().fmul_latency, 4);
        assert_eq!(Znver2.tune().fmul_latency, 3);
        // VFMADD231PS ymm.
        assert_eq!(Haswell.tune().fma_latency, 5);
        assert_eq!(Skylake.tune().fma_latency, 4);
        assert_eq!(RaptorLake.tune().fma_latency, 4);
        assert_eq!(Gracemont.tune().fma_latency, 6);
        assert_eq!(RaptorLake.tune().ecore.unwrap().fma_latency, 6);
        assert_eq!(ArrowLake.tune().ecore.unwrap().fma_latency, 4);
        assert_eq!(Znver2.tune().fma_latency, 5);
        assert_eq!(Znver3.tune().fma_latency, 4);
        assert_eq!(SandyBridge.tune().fma_latency, 0);
    }

    #[test]
    fn raptorlake_differs_from_alderlake_only_where_hardware_does() {
        let adl = X86Cpu::AlderLake.tune();
        let rpl = X86Cpu::RaptorLake.tune();
        // Same core: every instruction-level number is identical.
        let same = |a: &X86Tune| {
            format!(
                "{} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {:?}",
                a.dispatch_width,
                a.mispredict_penalty,
                a.rob_entries,
                a.lsd_uops,
                a.uop_cache_uops,
                a.popcnt_false_dep,
                a.lzcnt_tzcnt_false_dep,
                a.shift_cl_uops,
                a.lea3_latency,
                a.cmov_uops,
                a.imul64_latency,
                a.div64_latency,
                a.fma_latency,
                a.fadd_latency,
                a.fmul_latency,
                a.erms,
                a.fsrm,
                a.rep_movsb_threshold,
                a.rep_stosb_threshold,
                a.load_latency,
                a.cache.l1d,
            )
        };
        assert_eq!(same(&adl), same(&rpl));
        // Different uncore: L2 2 MiB/16-way, bigger L3, 4 MiB E-core L2,
        // more cycles per DRAM access at the higher clock.
        assert_eq!(rpl.cache.l2.kib, 2048);
        assert_eq!(rpl.cache.l2.ways, 16);
        assert_eq!(adl.cache.l2.kib, 1280);
        assert!(rpl.cache.l3.kib > adl.cache.l3.kib);
        assert_eq!(rpl.ecore.unwrap().cluster_l2_kib, 4096);
        assert_eq!(adl.ecore.unwrap().cluster_l2_kib, 2048);
        assert!(rpl.cache.dram_latency_cycles > adl.cache.dram_latency_cycles);
        assert_eq!(rpl.on_core_bytes(), 2 << 20);
        // The bigger L3 moves the non-temporal / libcall bound up.
        assert!(rpl.libcall_above_bytes() > adl.libcall_above_bytes());
        assert_eq!(rpl.libcall_above_bytes(), 33792 * 1024 / 4);
    }

    #[test]
    fn hybrid_rows_carry_gracemont_and_ecore_only_parts_use_it() {
        let e = X86Cpu::RaptorLake.tune().ecore.expect("hybrid");
        let g = X86Cpu::Gracemont.tune();
        assert_eq!(e.lea3_latency, g.lea3_latency);
        assert_eq!(e.shift_cl_uops, g.shift_cl_uops);
        assert_eq!(e.imul64_latency, g.imul64_latency);
        assert_eq!(e.div64_latency, g.div64_latency);
        assert_eq!(e.fma_latency, g.fma_latency);
        assert_eq!(e.load_latency, g.load_latency);
        assert!(X86Cpu::IceLake.tune().ecore.is_none());
        assert!(X86Cpu::Znver4.tune().ecore.is_none());
        assert_eq!(X86Cpu::from_name("sierraforest"), Some(X86Cpu::Gracemont));
        assert_eq!(X86Cpu::from_name("gracemont"), Some(X86Cpu::Gracemont));
        assert_eq!(X86Cpu::from_name("alderlake"), Some(X86Cpu::AlderLake));
        assert_eq!(X86Cpu::from_name("meteorlake"), Some(X86Cpu::RaptorLake));
    }

    #[test]
    fn generic_is_the_conservative_envelope_and_preserves_defaults() {
        let g = X86Tune::GENERIC;
        for cpu in X86Cpu::ALL {
            let t = cpu.tune();
            assert!(g.popcnt_false_dep >= t.popcnt_false_dep);
            assert!(g.lzcnt_tzcnt_false_dep >= t.lzcnt_tzcnt_false_dep);
            assert!(g.rob_entries <= t.rob_entries, "{:?}", cpu);
            assert!(g.uop_cache_uops <= t.uop_cache_uops, "{:?}", cpu);
            assert!(g.dispatch_width <= t.dispatch_width, "{:?}", cpu);
            assert!(g.fma_pipes <= t.fma_pipes.max(1), "{:?}", cpu);
            assert!(g.fadd_pipes <= t.fadd_pipes, "{:?}", cpu);
            assert!(g.vec_int_alu_pipes <= t.vec_int_alu_pipes, "{:?}", cpu);
            assert!(g.simd_datapath_bits <= t.simd_datapath_bits, "{:?}", cpu);
            assert!(
                u32::from(g.vec_load_ports) * u32::from(g.vec_load_port_bits)
                    <= u32::from(t.vec_load_ports) * u32::from(t.vec_load_port_bits),
                "{:?}",
                cpu
            );
            assert!(g.fma_latency >= t.fma_latency, "{:?}", cpu);
            assert!(g.load_latency >= t.load_latency, "{:?}", cpu);
            assert!(g.cache.l2.kib <= t.cache.l2.kib, "{:?}", cpu);
            assert!(g.div64_latency >= t.div64_latency, "{:?}", cpu);
            assert!(!g.erms && !g.fsrm);
        }
        // Pre-model defaults that untuned builds must keep.
        assert_eq!(g.if_convert_arm_budget(), 8);
        assert_eq!(g.issue_width(), 4);
        assert_eq!(g.memcpy_strategy(1 << 20, 32), CopyStrategy::InlineLoop);
        assert_eq!(g.memcpy_strategy(256, 32), CopyStrategy::InlineUnrolled);
        // IMUL deliberately NOT pessimistic (module docs): budget 2 = the
        // GCC/LLVM generic 2-instruction decomposition.
        assert_eq!(g.mul_const_op_budget(), 2);
        assert_eq!(g.libcall_above_bytes(), 8192);
    }

    #[test]
    fn memcpy_strategy_follows_glibc_thresholds() {
        use X86Cpu::*;
        let rpl = RaptorLake.tune();
        assert_eq!(rpl.memcpy_strategy(256, 32), CopyStrategy::InlineUnrolled);
        assert_eq!(rpl.memcpy_strategy(1024, 32), CopyStrategy::InlineLoop);
        assert_eq!(rpl.memcpy_strategy(2111, 32), CopyStrategy::InlineLoop);
        assert_eq!(rpl.memcpy_strategy(2112, 32), CopyStrategy::RepMovsb);
        assert_eq!(rpl.memcpy_strategy(4096, 32), CopyStrategy::RepMovsb);
        assert_eq!(rpl.memcpy_strategy(128, 16), CopyStrategy::InlineUnrolled);
        assert_eq!(rpl.memcpy_strategy(129, 16), CopyStrategy::InlineLoop);
        // Struct copies never libcall; calls do above L3/4.
        assert_eq!(rpl.memcpy_strategy(64 << 20, 32), CopyStrategy::RepMovsb);
        assert_eq!(rpl.memcpy_call_strategy(64 << 20, 32), CopyStrategy::LibCall);
        assert_eq!(rpl.memcpy_call_strategy(1 << 20, 32), CopyStrategy::RepMovsb);
        let skl = Skylake.tune();
        assert_eq!(skl.memcpy_strategy(4096, 32), CopyStrategy::InlineLoop);
        assert_eq!(skl.memcpy_strategy(8192, 32), CopyStrategy::RepMovsb);
        // No ERMS on Sandy Bridge: never rep movsb, 16-byte vectors, GCC's
        // 8 KiB libcall bound for calls.
        let snb = SandyBridge.tune();
        assert_eq!(snb.memcpy_strategy(1 << 20, 32), CopyStrategy::InlineLoop);
        assert_eq!(snb.block_copy_vector_bytes(true), 16);
        assert_eq!(snb.memcpy_call_strategy(8193, 16), CopyStrategy::LibCall);
        assert_eq!(snb.memcpy_call_strategy(8192, 16), CopyStrategy::InlineLoop);
        assert_eq!(Haswell.tune().block_copy_vector_bytes(true), 32);
        assert_eq!(Haswell.tune().block_copy_vector_bytes(false), 16);
        assert_eq!(Znver3.tune().memcpy_strategy(8192, 32), CopyStrategy::RepMovsb);
        assert_eq!(Znver4.tune().memcpy_strategy(2112, 32), CopyStrategy::RepMovsb);
    }

    #[test]
    fn memset_strategy_follows_glibc_rep_stosb_threshold() {
        use X86Cpu::*;
        let rpl = RaptorLake.tune();
        assert_eq!(rpl.memset_strategy(256, 32), CopyStrategy::InlineUnrolled);
        assert_eq!(rpl.memset_strategy(1024, 32), CopyStrategy::InlineLoop);
        assert_eq!(rpl.memset_strategy(2047, 32), CopyStrategy::InlineLoop);
        assert_eq!(rpl.memset_strategy(2048, 32), CopyStrategy::RepMovsb);
        assert_eq!(rpl.memset_strategy(4096, 32), CopyStrategy::RepMovsb);
        assert_eq!(rpl.memset_strategy(64 << 20, 32), CopyStrategy::LibCall);
        // Same 2048 on ERMS-only parts (glibc has no FSRM split for stores).
        assert_eq!(Skylake.tune().memset_strategy(2048, 32), CopyStrategy::RepMovsb);
        assert_eq!(Znver3.tune().memset_strategy(2048, 32), CopyStrategy::RepMovsb);
        let g = X86Tune::GENERIC;
        assert_eq!(g.memset_strategy(4096, 32), CopyStrategy::InlineLoop);
        assert_eq!(g.memset_strategy(8193, 32), CopyStrategy::LibCall);
        assert_eq!(SandyBridge.tune().memset_strategy(4096, 16), CopyStrategy::InlineLoop);
    }

    #[test]
    fn mul_const_plans_match_gcc_llvm_on_generic_and_go_further_on_gracemont() {
        use MulStep::*;
        let g = X86Tune::GENERIC;
        let steps = |k: i64| g.mul_const_plan(k).map(|p| p.steps().to_vec());
        assert_eq!(steps(0), None);
        assert_eq!(steps(1), None);
        assert_eq!(steps(2), Some(vec![Shl(1)]));
        assert_eq!(steps(8), Some(vec![Shl(3)]));
        assert_eq!(steps(3), Some(vec![LeaMul(2)]));
        assert_eq!(steps(5), Some(vec![LeaMul(4)]));
        assert_eq!(steps(9), Some(vec![LeaMul(8)]));
        assert_eq!(steps(6), Some(vec![LeaMul(2), Shl(1)]));
        assert_eq!(steps(10), Some(vec![LeaMul(4), Shl(1)]));
        assert_eq!(steps(12), Some(vec![LeaMul(2), Shl(2)]));
        assert_eq!(steps(24), Some(vec![LeaMul(2), Shl(3)]));
        assert_eq!(steps(36), Some(vec![LeaMul(8), Shl(2)]));
        assert_eq!(steps(40), Some(vec![LeaMul(4), Shl(3)]));
        assert_eq!(steps(15), Some(vec![LeaMul(2), LeaMul(4)]));
        assert_eq!(steps(25), Some(vec![LeaMul(4), LeaMul(4)]));
        assert_eq!(steps(27), Some(vec![LeaMul(2), LeaMul(8)]));
        assert_eq!(steps(45), Some(vec![LeaMul(4), LeaMul(8)]));
        assert_eq!(steps(81), Some(vec![LeaMul(8), LeaMul(8)]));
        assert_eq!(steps(7), Some(vec![LeaScale(8), SubSrc]));
        assert_eq!(steps(31), Some(vec![Shl(5), SubSrc]));
        assert_eq!(steps(17), Some(vec![Shl(4), AddSrc]));
        assert_eq!(steps(65), Some(vec![Shl(6), AddSrc]));
        assert_eq!(steps(-3), Some(vec![LeaMul(2), Neg]));
        assert_eq!(steps(-8), Some(vec![Shl(3), Neg]));
        // 3-op shapes stay IMUL on a 3-cycle multiplier.
        for k in [11, 13, 14, 19, 21, 22, 23, 26, 28, 29, 30, 50, 100, 1000, -7, -10, -100] {
            assert_eq!(steps(k), None, "k={k}");
        }
        // Register constraint is reported.
        assert!(g.mul_const_plan(7).unwrap().needs_distinct_src);
        assert!(!g.mul_const_plan(15).unwrap().needs_distinct_src);
        // Every P-core and Zen row agrees with Generic.
        for cpu in X86Cpu::ALL {
            let t = cpu.tune();
            if cpu == X86Cpu::Gracemont {
                continue;
            }
            assert_eq!(t.mul_const_op_budget(), 2, "{:?}", cpu);
            for k in [-100, -10, 6, 7, 10, 15, 17, 31, 100, 1000] {
                assert_eq!(t.mul_const_plan(k), g.mul_const_plan(k), "{:?} k={k}", cpu);
            }
        }
        // Gracemont (IMUL 5): up to 4 one-cycle steps beat the multiplier.
        let e = X86Cpu::Gracemont.tune();
        assert_eq!(e.mul_const_op_budget(), 4);
        let es = |k: i64| e.mul_const_plan(k).map(|p| p.steps().to_vec());
        assert_eq!(es(100), Some(vec![LeaMul(4), LeaMul(4), Shl(2)]));
        assert_eq!(es(14), Some(vec![LeaScale(8), SubSrc, Shl(1)]));
        assert_eq!(es(-10), Some(vec![LeaMul(4), Shl(1), Neg]));
        assert_eq!(es(-100), Some(vec![LeaMul(4), LeaMul(4), Shl(2), Neg]));
        assert_eq!(es(11), None);
        assert_eq!(es(1000), None);
        // Plans are never as long as IMUL itself.
        for cpu in X86Cpu::ALL {
            let t = cpu.tune();
            for k in -200i64..=200 {
                if let Some(p) = t.mul_const_plan(k) {
                    assert!(p.len() < t.imul64_latency as usize, "{:?} k={k} {:?}", cpu, p);
                    assert!(p.len() <= 4);
                }
            }
        }
    }

    #[test]
    fn mul_const_plans_compute_the_right_product() {
        // Interpret every plan for every row over a set of multiplicands
        // and check it against wrapping multiplication.
        fn run(p: &MulPlan, src: i64) -> i64 {
            let mut acc = src;
            for (i, s) in p.steps().iter().enumerate() {
                acc = match *s {
                    MulStep::LeaScale(sc) => {
                        assert_eq!(i, 0, "LeaScale must be first");
                        src.wrapping_mul(sc as i64)
                    }
                    MulStep::LeaMul(sc) => acc.wrapping_add(acc.wrapping_mul(sc as i64)),
                    MulStep::Shl(k) => acc.wrapping_shl(k as u32),
                    MulStep::AddSrc => acc.wrapping_add(src),
                    MulStep::SubSrc => acc.wrapping_sub(src),
                    MulStep::Neg => acc.wrapping_neg(),
                };
            }
            acc
        }
        let inputs = [0i64, 1, -1, 2, 3, 7, 12345, -98765, i64::MAX, i64::MIN, 0x7fff_ffff, -0x8000_0000];
        for cpu in X86Cpu::ALL {
            let t = cpu.tune();
            for k in (-1100i64..=1100).chain([i32::MAX as i64, i32::MIN as i64]) {
                if let Some(p) = t.mul_const_plan(k) {
                    for &x in &inputs {
                        assert_eq!(run(&p, x), x.wrapping_mul(k), "{:?} k={k} x={x} {:?}", cpu, p);
                    }
                }
            }
        }
    }

    #[test]
    fn if_convert_budget_tracks_mispredict_penalty() {
        use X86Cpu::*;
        for cpu in [Skylake, IceLake, AlderLake, RaptorLake, Generic] {
            assert_eq!(cpu.tune().if_convert_arm_budget(), 8, "{:?}", cpu);
        }
        for cpu in [SandyBridge, Haswell] {
            assert_eq!(cpu.tune().if_convert_arm_budget(), 7, "{:?}", cpu);
        }
        assert_eq!(Znver1.tune().if_convert_arm_budget(), 9);
        assert_eq!(Znver3.tune().if_convert_arm_budget(), 6);
        assert_eq!(Gracemont.tune().if_convert_arm_budget(), 6);
        for cpu in X86Cpu::ALL {
            let b = cpu.tune().if_convert_arm_budget();
            assert!((4..=12).contains(&b), "{:?} budget {}", cpu, b);
        }
    }

    #[test]
    fn issue_width_matches_vendor_documentation() {
        use X86Cpu::*;
        assert_eq!(Skylake.tune().issue_width(), 4);
        assert_eq!(IceLake.tune().issue_width(), 5);
        assert_eq!(RaptorLake.tune().issue_width(), 6);
        assert_eq!(ArrowLake.tune().issue_width(), 8);
        assert_eq!(Znver3.tune().issue_width(), 6);
        assert_eq!(Znver5.tune().issue_width(), 8);
    }

    #[test]
    fn resolve_precedence_and_native() {
        assert_eq!(resolve(None, None).cpu, X86Cpu::Generic);
        assert_eq!(resolve(Some("skylake"), None).cpu, X86Cpu::Skylake);
        assert_eq!(resolve(Some("skylake"), Some("znver3")).cpu, X86Cpu::Znver3);
        assert_eq!(resolve(Some("x86-64-v3"), None).cpu, X86Cpu::Generic);
        assert_eq!(resolve(None, Some("raptorlake")).cpu, X86Cpu::RaptorLake);
        assert_eq!(resolve(Some("raptorlake"), None).cpu, X86Cpu::RaptorLake);
        assert_eq!(resolve(None, Some("no-such-cpu")).cpu, X86Cpu::Generic);
        let n = resolve(None, Some("native"));
        assert!(X86Cpu::ALL.contains(&n.cpu));
        // Cache geometry from CPUID must be sane whenever it is reported.
        assert!(n.cache.l1d.kib >= 8 && n.cache.l1d.kib <= 256, "{:?}", n.cache);
        assert!(n.cache.line_bytes == 32 || n.cache.line_bytes == 64 || n.cache.line_bytes == 128);
        assert!(n.cache.l2.kib >= n.cache.l1d.kib);
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
        assert_eq!(intel(0x9A), AlderLake);
        assert_eq!(intel(0xB7), RaptorLake); // Raptor Lake-S (i7-14700KF)
        assert_eq!(intel(0xBA), RaptorLake); // Raptor Lake-P
        assert_eq!(intel(0xBF), RaptorLake);
        assert_eq!(intel(0xAA), RaptorLake); // Meteor Lake
        assert_eq!(intel(0xBE), Gracemont); // Alder Lake-N
        assert_eq!(intel(0xAF), Gracemont); // Sierra Forest
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
        assert_eq!(X86Cpu::RaptorLake.tune().fma_reduction_accumulators(), 8);
        assert_eq!(X86Cpu::Znver1.tune().fma_reduction_accumulators(), 5);
        assert_eq!(X86Cpu::Znver3.tune().fma_reduction_accumulators(), 8);
        assert_eq!(X86Cpu::SandyBridge.tune().fma_reduction_accumulators(), 0);
        // Gracemont: two 128-bit FMA pipes → one 256-bit FMA per cycle.
        assert_eq!(X86Cpu::Gracemont.tune().fma_reduction_accumulators(), 6);
        // Golden Cove's 2-cycle adders halve the FADD chain split vs SKL.
        assert_eq!(X86Cpu::AlderLake.tune().fadd_reduction_accumulators(), 4);
        assert_eq!(X86Cpu::Skylake.tune().fadd_reduction_accumulators(), 8);
        // [uops.info] VADDPS ymm on HSW/BDW is p1 only: one adder, not two.
        assert_eq!(X86Cpu::Haswell.tune().fadd_reduction_accumulators(), 3);
        assert_eq!(X86Cpu::Broadwell.tune().fadd_reduction_accumulators(), 3);
        assert_eq!(X86Cpu::SandyBridge.tune().fadd_reduction_accumulators(), 3);
        // Zen1 physical pipes are 2 × 128: 256-bit FADD → 3 × 1.
        assert_eq!(X86Cpu::Znver1.tune().fadd_reduction_accumulators(), 3);
        assert_eq!(X86Cpu::Znver3.tune().fadd_reduction_accumulators(), 6);
    }

    #[test]
    fn simd_datapath_and_load_ports_are_consistent() {
        for cpu in X86Cpu::ALL {
            let t = cpu.tune();
            assert!(matches!(t.simd_datapath_bits, 128 | 256 | 512), "{:?}", cpu);
            assert!(matches!(t.vec_load_port_bits, 128 | 256 | 512), "{:?}", cpu);
            assert!((1..=4).contains(&t.vec_load_ports), "{:?}", cpu);
            assert!((1..=4).contains(&t.fadd_pipes), "{:?}", cpu);
            assert!((2..=4).contains(&t.vec_int_alu_pipes), "{:?}", cpu);
            // A load port is never narrower than the datapath it feeds by
            // more than one halving (SNB: 256-bit FP, 128-bit load ports).
            assert!(t.vec_load_port_bits * 2 >= t.simd_datapath_bits, "{:?}", cpu);
        }
        let rpl = X86Cpu::RaptorLake.tune();
        assert_eq!(rpl.vector_loads_per_cycle(256), 3.0);
        assert_eq!(rpl.simd_pipes_for(2, 256), 2.0);
        assert_eq!(rpl.prefer_vector_bits(), 256);
        let gm = X86Cpu::Gracemont.tune();
        assert_eq!(gm.vector_loads_per_cycle(256), 1.0);
        assert_eq!(gm.simd_pipes_for(2, 256), 1.0);
        assert_eq!(gm.prefer_vector_bits(), 128);
        let z1 = X86Cpu::Znver1.tune();
        assert_eq!(z1.prefer_vector_bits(), 128);
        assert_eq!(z1.simd_pipes_for(2, 256), 1.0);
        assert_eq!(X86Cpu::SkylakeAvx512.tune().prefer_vector_bits(), 512);
        assert_eq!(X86Cpu::Znver4.tune().prefer_vector_bits(), 256);
        assert_eq!(X86Cpu::Znver5.tune().prefer_vector_bits(), 512);
        assert_eq!(X86Cpu::Generic.tune().prefer_vector_bits(), 128);
    }

    #[test]
    fn reduction_interleave_follows_the_roofline() {
        let dot = ReductionShape {
            op: ReductionOp::Fma,
            vector_bits: 256,
            accumulators: 1,
            loads_per_iter: 2,
            uops_per_iter: 2,
        };
        let sum_f = ReductionShape {
            op: ReductionOp::FAdd,
            vector_bits: 256,
            accumulators: 1,
            loads_per_iter: 1,
            uops_per_iter: 1,
        };
        let sum_i = ReductionShape {
            op: ReductionOp::IntAdd,
            ..sum_f
        };
        let max_i = ReductionShape {
            op: ReductionOp::IntMax,
            ..sum_f
        };
        // FMA dot product: Skylake is load-port bound (2 loads/cycle, 2 per
        // FMA → 1 FMA/cycle) and 4 chains cover a 4-cycle latency exactly.
        assert_eq!(X86Cpu::Skylake.tune().reduction_interleave(dot), 4);
        // Haswell's FMA is 5 cycles: 4 chains reach only 0.8 FMA/cycle of
        // the same 1.0 load bound, so it needs 5 → 8 chains (Broadwell too).
        assert_eq!(X86Cpu::Haswell.tune().reduction_interleave(dot), 8);
        assert_eq!(X86Cpu::Broadwell.tune().reduction_interleave(dot), 8);
        // Raptor Lake / Alder Lake / Sapphire Rapids: three load ports lift
        // the bound to 1.5 FMA/cycle, which needs 6 → 8 chains.
        assert_eq!(X86Cpu::RaptorLake.tune().reduction_interleave(dot), 8);
        assert_eq!(X86Cpu::AlderLake.tune().reduction_interleave(dot), 8);
        assert_eq!(X86Cpu::SapphireRapids.tune().reduction_interleave(dot), 8);
        // Zen3/4: two 256-bit loads/cycle → 1 FMA/cycle at latency 4 → 4.
        assert_eq!(X86Cpu::Znver3.tune().reduction_interleave(dot), 4);
        assert_eq!(X86Cpu::Znver4.tune().reduction_interleave(dot), 4);
        // Zen1: 256-bit loads at 1/cycle → 0.5 FMA/cycle × latency 5 → 4.
        assert_eq!(X86Cpu::Znver1.tune().reduction_interleave(dot), 4);
        // SNB (no FMA): mul + add chains on one adder, one 256-bit load per
        // cycle → the LLVM floor of 4.
        assert_eq!(X86Cpu::SandyBridge.tune().reduction_interleave(dot), 4);
        // FP sum: one load per add.  SKL adds at latency 4 on two pipes with
        // two loads/cycle → 8 chains; Golden Cove's 2-cycle adders need 4.
        assert_eq!(X86Cpu::Skylake.tune().reduction_interleave(sum_f), 8);
        assert_eq!(X86Cpu::RaptorLake.tune().reduction_interleave(sum_f), 4);
        assert_eq!(X86Cpu::Znver3.tune().reduction_interleave(sum_f), 8);
        // Haswell: a single adder → 1 add/cycle × latency 3 → the floor 4.
        assert_eq!(X86Cpu::Haswell.tune().reduction_interleave(sum_f), 4);
        // Integer add / max (latency 1): never above the floor.
        for cpu in X86Cpu::ALL {
            assert_eq!(cpu.tune().reduction_interleave(sum_i), 4, "{:?}", cpu);
            assert_eq!(cpu.tune().reduction_interleave(max_i), 4, "{:?}", cpu);
        }
        // 128-bit shapes: floor 2, SKL FP sum still wants 8 (lat 4 × 2 pipes
        // = 8 at 2 loads/cycle).
        let sum_f128 = ReductionShape {
            vector_bits: 128,
            ..sum_f
        };
        assert_eq!(X86Cpu::Skylake.tune().reduction_interleave(sum_f128), 8);
        // Generic: one adder on a 128-bit datapath → 1 add/cycle × lat 4 → 4.
        assert_eq!(X86Cpu::Generic.tune().reduction_interleave(sum_f128), 4);
        let sum_i128 = ReductionShape {
            vector_bits: 128,
            ..sum_i
        };
        assert_eq!(X86Cpu::Generic.tune().reduction_interleave(sum_i128), 2);
        // Raptor Lake: three 128-bit loads and three vector ALUs per cycle
        // sustain 3 adds/cycle, so 2 chains leave a third of it idle → 4.
        assert_eq!(X86Cpu::RaptorLake.tune().reduction_interleave(sum_i128), 4);
        // Register cap: two accumulator phis share the 8-register budget.
        let dot2 = ReductionShape {
            accumulators: 2,
            loads_per_iter: 4,
            uops_per_iter: 4,
            ..dot
        };
        assert_eq!(X86Cpu::RaptorLake.tune().reduction_interleave(dot2), 4);
        let dot4 = ReductionShape {
            accumulators: 4,
            loads_per_iter: 8,
            uops_per_iter: 8,
            ..dot
        };
        assert_eq!(X86Cpu::RaptorLake.tune().reduction_interleave(dot4), 2);
        // Generic (no -mtune): the envelope sustains one 256-bit load per
        // cycle (2 × 128-bit ports), so every 256-bit shape is load-bound by
        // 4 chains — the historical constant is reproduced exactly.
        assert_eq!(X86Cpu::Generic.tune().reduction_interleave(sum_f), 4);
        assert_eq!(X86Cpu::Generic.tune().reduction_interleave(dot), 4);
        assert_eq!(X86Cpu::Generic.tune().reduction_interleave(sum_i), 4);
    }

    #[test]
    fn cache_rows_are_monotone_and_documented() {
        for cpu in X86Cpu::ALL {
            let c = cpu.tune().cache;
            assert_eq!(c.line_bytes, 64, "{:?}", cpu);
            assert!(c.l1d.kib < c.l2.kib && c.l2.kib < c.l3.kib, "{:?}", cpu);
            assert!(c.l1d.latency < c.l2.latency && c.l2.latency < c.l3.latency, "{:?}", cpu);
            assert!(c.l3.latency < 128 && (c.dram_latency_cycles as u32) > c.l3.latency as u32 * 2);
            assert_eq!(cpu.tune().load_latency, c.l1d.latency, "{:?}", cpu);
        }
        assert_eq!(X86Cpu::Skylake.tune().cache.l2.ways, 4);
        assert_eq!(X86Cpu::IceLake.tune().cache.l1d.kib, 48);
        assert_eq!(X86Cpu::Znver4.tune().cache.l2.kib, 1024);
        assert_eq!(X86Cpu::Znver5.tune().cache.l1d.kib, 48);
    }

    #[test]
    fn dump_is_stable_and_complete() {
        let d = X86Cpu::RaptorLake.tune().dump();
        assert!(d.starts_with("tune=raptorlake\n"));
        assert!(d.contains("popcnt_false_dep=false\n"));
        assert!(d.contains("shift_cl_uops=2\n"));
        assert!(d.contains("lsd_uops=144\n"));
        assert!(d.contains("fadd_latency=2\n"));
        assert!(d.contains("cache.l2_kib=2048\n"));
        assert!(d.contains("ecore=gracemont\n"));
        assert!(d.contains("ecore.cluster_l2_kib=4096\n"));
        assert!(d.contains("derived.fma_reduction_accumulators=8\n"));
        assert!(d.contains("derived.if_convert_arm_budget=8\n"));
        assert!(d.contains("derived.mul_const_op_budget=2\n"));
        assert!(d.contains("derived.mul_const_plan_100=Imul\n"));
        assert!(d.contains("derived.memcpy_4096_avx2=RepMovsb\n"));
        assert!(d.contains("derived.memset_4096_avx2=RepMovsb\n"));
        let g = X86Tune::GENERIC.dump();
        assert!(g.contains("ecore=none\n"));
        assert!(g.contains("derived.memcpy_4096_avx2=InlineLoop\n"));
        assert!(g.contains("derived.libcall_above_bytes=8192\n"));
        let e = X86Cpu::Gracemont.tune().dump();
        assert!(e.contains("derived.mul_const_op_budget=4\n"));
        assert!(e.contains("derived.mul_const_plan_100=[LeaMul(4), LeaMul(4), Shl(2)]\n"));
        // Every row dumps the same key set in the same order (scripts rely on it).
        let keys = |s: &str| {
            s.lines()
                .map(|l| l.split('=').next().unwrap().to_string())
                .filter(|k| !k.starts_with("ecore."))
                .collect::<Vec<_>>()
        };
        for cpu in X86Cpu::ALL {
            assert_eq!(keys(&cpu.tune().dump()), keys(&d), "{:?}", cpu);
        }
    }
}
