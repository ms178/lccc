//! Runtime x86 CPU probe for `__builtin_cpu_supports` (AUD-CPU-1).
//!
//! # Why this exists
//!
//! GCC lowers `__builtin_cpu_supports("f")` to a load from libgcc's
//! `__cpu_model` / `__cpu_features2`, i.e. it answers the question "does the
//! CPU executing this program have `f`?" — never a compile-time constant.
//! That is what every runtime dispatcher in the wild (zlib-ng, zstd, xz,
//! OpenSSL, FFmpeg) relies on, and what lets a *generic* build still take its
//! AVX2/SHA path on a CPU that has them.
//!
//! LCCC used to fold the builtin against the *effective target* feature set
//! (the old static fold in `crate::common::isa_features`).  That was wrong
//! twice over, and both errors were measured against GCC 16.2, Clang 23.1 and
//! ICX (see `scripts/godbolt.py` and the census in `tests/data/`):
//!
//! * `-march=`/`-mno-*` describe the code the compiler may *emit*, not what
//!   the CPU running the program supports: all three reference compilers keep
//!   emitting a runtime probe for `-march=x86-64-v3`, for
//!   `-march=raptorlake` and even under an explicit `-mno-avx2`.  A folded
//!   `1` therefore told every dispatcher "this CPU has AVX2" about a
//!   hypothetical baseline machine — the SIGILL class of bug the fold was
//!   supposed to prevent.
//! * a runtime builtin is not an integer constant expression, so GCC, Clang
//!   and ICX reject all four constant contexts (array bound, case label,
//!   static initializer, enumerator) in which a folded LCCC silently accepted
//!   it.
//!
//! The builtin is therefore always lowered to the runtime probe this module
//! generates — never to a constant — and those four contexts diagnose with
//! GCC's wording (`variably modified 'a' at file scope`, `case label does not
//! reduce to an integer constant`, `initializer element is not constant`,
//! `enumerator value for 'E' is not an integer constant`).
//!
//! Lowering emits the probe sites; the driver appends [`runtime_asm`] to the
//! translation unit once, right after code generation, when
//! [`take_runtime_needed`] says a site exists.
//!
//! # Semantics (identical to libgcc's runtime model)
//!
//! * The probe reads CPUID leaves 0, 1, 7 (subleaves 0 and 1), 0xD (subleaf 1),
//!   0x14 (subleaf 0), 0x24 (subleaf 0) and 0x80000001/0x80000008, each
//!   guarded by the CPU's advertised maximum leaf (basic or extended), so no
//!   undefined CPUID input is ever issued.
//! * `AVX`-class features additionally require OSXSAVE (`CPUID.1:ECX[27]`) and
//!   `XGETBV(0) & 0x6 == 0x6` (XMM+YMM state enabled by the OS); AVX-512-class
//!   features require `XGETBV(0) & 0xE6 == 0xE6`; AMX-class features require
//!   `XGETBV(0) & 0x60000 == 0x60000`.  A CPU whose OS does not save the state
//!   cannot legally execute the instruction no matter what CPUID says, which is
//!   why the xstate gate clears the feature bit in the cached word (the shape
//!   libgcc uses when it builds `__cpu_features`).
//! * The cached words are zero-initialised, so a probe that ran before
//!   initialisation (impossible today: the init call is hoisted into the entry
//!   block of every function that probes) would answer `0` — the safe
//!   direction, never a false `1`.
//!
//! # Verification
//!
//! The CPUID bit assignment of every name in [`PROBE_TABLE`] was checked against
//! two independent reference implementations (GCC's libgcc `cpuinfo.c` model and
//! LLVM's compiler-rt `cpu_model/x86.c`) and, for every feature the development
//! host implements (54 of them, including the whole AVX-512 and SHA/GFNI/VAES
//! groups), against the values GCC's own runtime model returns on that host:
//! **zero mismatches**.  `tests/regression/check_cpu_supports_runtime.sh` keeps
//! that property on every CI run by differentially executing LCCC-built and
//! GCC-built probes over the whole accepted name set; the accepted name set
//! itself is pinned in `tests/data/cpu_supports_name_census.json`, regenerated
//! by `scripts/cpu_supports_name_census.py`.
//!
//! # Deliberate deviations from the reference implementations
//!
//! * `xop`, `fma4`, `tbm` and `gfni` are gated on the VEX (YMM) xstate here,
//!   while compiler-rt does not gate them.  All four are VEX-encoded, so an OS
//!   that does not save YMM state cannot run them either; the gated answer is
//!   the conservative one (a dispatcher takes its generic path) and is
//!   unobservable on any CPU that can execute the instruction at all.
//! * The answer is `0`/`1` (`1<<bit` masked down), not libgcc's raw feature bit
//!   mask.  C truthiness is identical, and `int x = ...; if (x == 1)` keeps
//!   working with LCCC where it would not with GCC.
//! * Names GCC rejects but LCCC's driver models (`avx512er`, `avx512pf`) stay
//!   accepted: they have CPUID bits and a `-m` flag, so rejecting them would be
//!   a gratuitous incompatibility — a superset of GCC's name set, never a
//!   subset.  Conversely `xsaveopt`/`xsavec`/`xsaves` (leaf 0xD) are probed
//!   although the driver has no `-m` flag for them, so GCC dispatch code using
//!   them works unchanged.

/// Target the generated helper is emitted for.
///
/// Deliberately a local enum rather than `crate::backend::Target`: this module
/// belongs to the lowest layer of the crate (`src/common/`), which does not
/// depend on the backends, and the driver maps its own target onto this.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProbeTarget {
    /// x86-64: RIP-relative references, 64-bit `push`/`pop` of `%rbx`.
    X86_64,
    /// i686: a PC base popped into `%ecx`, so the helper is position
    /// independent without a GOT and without a text relocation.
    I686,
}

/// Which cached CPUID/XSTATE word a feature bit lives in.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum CpuWord {
    /// `CPUID.1:EDX` — pre-SSE generations (CMOV, MMX, FXSR, SSE, SSE2).
    Leaf1Edx,
    /// `CPUID.1:ECX` — SSE3..AVX2-era scalar/vector bits.
    Leaf1Ecx,
    /// `CPUID.(7,0):EBX` — BMI, AVX2, AVX-512 F/DQ/BW/VL, SHA, ...
    Leaf7Ebx,
    /// `CPUID.(7,0):ECX` — AVX-512 VBMI/VBMI2/VNNI/BITALG, GFNI, VAES, ...
    Leaf7Ecx,
    /// `CPUID.(7,0):EDX` — AMX, AVX512-FP16, SERIALIZE, TSXLDTRK, ...
    Leaf7Edx,
    /// `CPUID.(7,1):EAX` — SHA512, SM3/SM4, AVX-VNNI, AVX512-BF16, ...
    Leaf71Eax,
    /// `CPUID.(7,1):EDX` — AVX-VNNI-INT8/16, AVX-NE-CONVERT.
    Leaf71Edx,
    /// `CPUID.(0xD,1):EAX` — XSAVEOPT/XSAVEC/XSAVES (bits 0/1/3).
    LeafD1Eax,
    /// `CPUID.0x80000001:ECX` — LZCNT/ABM, SSE4A, XOP, FMA4, TBM.
    Ext1Ecx,
    /// `CPUID.0x80000001:EDX` — long mode and other AMD/Intel extension bits.
    Ext1Edx,
    /// `CPUID.0x14:EBX` (subleaf 0) — Intel Processor Trace; PTWRITE is bit 4.
    Leaf14Ebx,
    /// `CPUID.0x80000008:EBX` — AMD cache/ISA bits: CLZERO (bit 0) and
    /// WBNOINVD (bit 9).
    Ext8Ebx,
    /// Synthetic word holding the AVX10 levels: bit *N* set means the CPU
    /// implements AVX10.*N* (`CPUID.(0x24,0):EBX[7:0] >= N`, gated by the AVX10
    /// advertisement bit `CPUID.(7,1):EDX[19]`).  Storing a *level* as a bit
    /// keeps every `avx10.N` probe an ordinary "load word, AND mask, test"
    /// and keeps the version comparison in the once-per-program initialiser.
    Avx10Levels,
    /// `XGETBV(0)` low half — which CPU state the OS has enabled.
    Xcr0,
}

impl CpuWord {
    /// Every word the generated probe maintains, for table-driven generation.
    pub const ALL: [CpuWord; 14] = [
        CpuWord::Leaf1Edx,
        CpuWord::Leaf1Ecx,
        CpuWord::Leaf7Ebx,
        CpuWord::Leaf7Ecx,
        CpuWord::Leaf7Edx,
        CpuWord::Leaf71Eax,
        CpuWord::Leaf71Edx,
        CpuWord::LeafD1Eax,
        CpuWord::Ext1Ecx,
        CpuWord::Ext1Edx,
        CpuWord::Leaf14Ebx,
        CpuWord::Ext8Ebx,
        CpuWord::Avx10Levels,
        CpuWord::Xcr0,
    ];

    /// The assembler symbol holding this word (a `.local`, zero-initialised
    /// `.bss` slot inside the translation unit that needed the probe).
    pub fn symbol(self) -> &'static str {
        match self {
            CpuWord::Leaf1Edx => "__lccc_cpu_w1edx",
            CpuWord::Leaf1Ecx => "__lccc_cpu_w1ecx",
            CpuWord::Leaf7Ebx => "__lccc_cpu_w7ebx",
            CpuWord::Leaf7Ecx => "__lccc_cpu_w7ecx",
            CpuWord::Leaf7Edx => "__lccc_cpu_w7edx",
            CpuWord::Leaf71Eax => "__lccc_cpu_w71eax",
            CpuWord::Leaf71Edx => "__lccc_cpu_w71edx",
            CpuWord::LeafD1Eax => "__lccc_cpu_wd1eax",
            CpuWord::Ext1Ecx => "__lccc_cpu_wext1ecx",
            CpuWord::Ext1Edx => "__lccc_cpu_wext1edx",
            CpuWord::Leaf14Ebx => "__lccc_cpu_w14ebx",
            CpuWord::Ext8Ebx => "__lccc_cpu_wext8ebx",
            CpuWord::Avx10Levels => "__lccc_cpu_wavx10",
            CpuWord::Xcr0 => "__lccc_cpu_wxcr0",
        }
    }
}

/// The OS-enabled xstate a feature additionally requires.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Xstate {
    /// No xstate gate (integer features, and legacy SSE encodings).
    None,
    /// XMM+YMM state: `XGETBV(0) & 0x6 == 0x6`.  Every VEX-encoded feature,
    /// plus the XSAVEOPT/XSAVEC/XSAVES management instructions.
    Vex,
    /// AVX-512 state: `XGETBV(0) & 0xE6 == 0xE6` (opmask + ZMM + Hi16 ZMM).
    Avx512,
    /// AMX state: `XGETBV(0) & 0x60000 == 0x60000` (bits 17 and 18).
    Amx,
}

impl Xstate {
    /// The bits that must all be set in `XGETBV(0)`.
    pub fn mask(self) -> u32 {
        match self {
            Xstate::None => 0,
            Xstate::Vex => 0x6,
            Xstate::Avx512 => 0xE6,
            Xstate::Amx => 0x60000,
        }
    }

    /// Human-readable name used in the generated assembly's jump labels.
    pub fn label(self) -> &'static str {
        match self {
            Xstate::None => "none",
            Xstate::Vex => "vex",
            Xstate::Avx512 => "avx512",
            Xstate::Amx => "amx",
        }
    }
}

/// One feature's runtime location: `(word, bit)` plus its xstate requirement.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Probe {
    /// Cached CPUID word the bit lives in.
    pub word: CpuWord,
    /// Bit number inside that word (0-31).
    pub bit: u8,
    /// xstate the OS must have enabled for the instruction to be legal.
    pub state: Xstate,
}

impl Probe {
    const fn new(word: CpuWord, bit: u8, state: Xstate) -> Self {
        Probe { word, bit, state }
    }

    /// The mask the lowering ANDs into the loaded word.
    pub fn bit_mask(self) -> u32 {
        1u32 << self.bit
    }
}

/// The runtime probe table: canonical feature name -> its CPUID location.
///
/// Names and CPUID assignments follow GCC's `__builtin_cpu_supports` (GCC's
/// accepted set is the compatibility target; see
/// `tests/data/cpu_supports_name_census.json`); the aliases handled by
/// [`lookup`] additionally cover Clang/Intel spellings (`pclmulqdq`,
/// `rdrand`) and the underscore spellings used by `-m` flags and macros.
pub const PROBE_TABLE: &[(&str, Probe)] = &[
    // ---- leaf 1 EDX / ECX ---------------------------------------------------
    ("cmov", Probe::new(CpuWord::Leaf1Edx, 15, Xstate::None)),
    ("fxsr", Probe::new(CpuWord::Leaf1Edx, 24, Xstate::None)),
    ("mmx", Probe::new(CpuWord::Leaf1Edx, 23, Xstate::None)),
    ("sse", Probe::new(CpuWord::Leaf1Edx, 25, Xstate::None)),
    ("sse2", Probe::new(CpuWord::Leaf1Edx, 26, Xstate::None)),
    ("sse3", Probe::new(CpuWord::Leaf1Ecx, 0, Xstate::None)),
    ("pclmul", Probe::new(CpuWord::Leaf1Ecx, 1, Xstate::None)),
    ("ssse3", Probe::new(CpuWord::Leaf1Ecx, 9, Xstate::None)),
    ("fma", Probe::new(CpuWord::Leaf1Ecx, 12, Xstate::Vex)),
    ("cx16", Probe::new(CpuWord::Leaf1Ecx, 13, Xstate::None)),
    ("sse4.1", Probe::new(CpuWord::Leaf1Ecx, 19, Xstate::None)),
    ("sse4.2", Probe::new(CpuWord::Leaf1Ecx, 20, Xstate::None)),
    ("movbe", Probe::new(CpuWord::Leaf1Ecx, 22, Xstate::None)),
    ("popcnt", Probe::new(CpuWord::Leaf1Ecx, 23, Xstate::None)),
    ("aes", Probe::new(CpuWord::Leaf1Ecx, 25, Xstate::None)),
    ("xsave", Probe::new(CpuWord::Leaf1Ecx, 26, Xstate::Vex)),
    ("osxsave", Probe::new(CpuWord::Leaf1Ecx, 27, Xstate::None)),
    ("avx", Probe::new(CpuWord::Leaf1Ecx, 28, Xstate::Vex)),
    ("f16c", Probe::new(CpuWord::Leaf1Ecx, 29, Xstate::Vex)),
    ("rdrnd", Probe::new(CpuWord::Leaf1Ecx, 30, Xstate::None)),
    // ---- leaf 7, subleaf 0 --------------------------------------------------
    ("fsgsbase", Probe::new(CpuWord::Leaf7Ebx, 0, Xstate::None)),
    ("sgx", Probe::new(CpuWord::Leaf7Ebx, 2, Xstate::None)),
    ("bmi", Probe::new(CpuWord::Leaf7Ebx, 3, Xstate::None)),
    ("hle", Probe::new(CpuWord::Leaf7Ebx, 4, Xstate::None)),
    ("avx2", Probe::new(CpuWord::Leaf7Ebx, 5, Xstate::Vex)),
    ("bmi2", Probe::new(CpuWord::Leaf7Ebx, 8, Xstate::None)),
    ("rtm", Probe::new(CpuWord::Leaf7Ebx, 11, Xstate::None)),
    ("rdseed", Probe::new(CpuWord::Leaf7Ebx, 18, Xstate::None)),
    ("adx", Probe::new(CpuWord::Leaf7Ebx, 19, Xstate::None)),
    (
        "clflushopt",
        Probe::new(CpuWord::Leaf7Ebx, 23, Xstate::None),
    ),
    ("clwb", Probe::new(CpuWord::Leaf7Ebx, 24, Xstate::None)),
    (
        "avx512pf",
        Probe::new(CpuWord::Leaf7Ebx, 26, Xstate::Avx512),
    ),
    (
        "avx512er",
        Probe::new(CpuWord::Leaf7Ebx, 27, Xstate::Avx512),
    ),
    ("sha", Probe::new(CpuWord::Leaf7Ebx, 29, Xstate::None)),
    ("avx512f", Probe::new(CpuWord::Leaf7Ebx, 16, Xstate::Avx512)),
    (
        "avx512dq",
        Probe::new(CpuWord::Leaf7Ebx, 17, Xstate::Avx512),
    ),
    (
        "avx512ifma",
        Probe::new(CpuWord::Leaf7Ebx, 21, Xstate::Avx512),
    ),
    (
        "avx512cd",
        Probe::new(CpuWord::Leaf7Ebx, 28, Xstate::Avx512),
    ),
    (
        "avx512bw",
        Probe::new(CpuWord::Leaf7Ebx, 30, Xstate::Avx512),
    ),
    (
        "avx512vl",
        Probe::new(CpuWord::Leaf7Ebx, 31, Xstate::Avx512),
    ),
    (
        "avx512vbmi",
        Probe::new(CpuWord::Leaf7Ecx, 1, Xstate::Avx512),
    ),
    ("pku", Probe::new(CpuWord::Leaf7Ecx, 4, Xstate::None)),
    ("waitpkg", Probe::new(CpuWord::Leaf7Ecx, 5, Xstate::None)),
    (
        "avx512vbmi2",
        Probe::new(CpuWord::Leaf7Ecx, 6, Xstate::Avx512),
    ),
    ("shstk", Probe::new(CpuWord::Leaf7Ecx, 7, Xstate::None)),
    ("gfni", Probe::new(CpuWord::Leaf7Ecx, 8, Xstate::Vex)),
    ("vaes", Probe::new(CpuWord::Leaf7Ecx, 9, Xstate::Vex)),
    ("vpclmulqdq", Probe::new(CpuWord::Leaf7Ecx, 10, Xstate::Vex)),
    (
        "avx512vnni",
        Probe::new(CpuWord::Leaf7Ecx, 11, Xstate::Avx512),
    ),
    (
        "avx512bitalg",
        Probe::new(CpuWord::Leaf7Ecx, 12, Xstate::Avx512),
    ),
    (
        "avx512vpopcntdq",
        Probe::new(CpuWord::Leaf7Ecx, 14, Xstate::Avx512),
    ),
    ("rdpid", Probe::new(CpuWord::Leaf7Ecx, 22, Xstate::None)),
    ("kl", Probe::new(CpuWord::Leaf7Ecx, 23, Xstate::None)),
    ("cldemote", Probe::new(CpuWord::Leaf7Ecx, 25, Xstate::None)),
    ("movdiri", Probe::new(CpuWord::Leaf7Ecx, 27, Xstate::None)),
    ("movdir64b", Probe::new(CpuWord::Leaf7Ecx, 28, Xstate::None)),
    ("enqcmd", Probe::new(CpuWord::Leaf7Ecx, 29, Xstate::None)),
    ("uintr", Probe::new(CpuWord::Leaf7Edx, 5, Xstate::None)),
    (
        "avx512vp2intersect",
        Probe::new(CpuWord::Leaf7Edx, 8, Xstate::Avx512),
    ),
    ("serialize", Probe::new(CpuWord::Leaf7Edx, 14, Xstate::None)),
    ("tsxldtrk", Probe::new(CpuWord::Leaf7Edx, 16, Xstate::None)),
    ("pconfig", Probe::new(CpuWord::Leaf7Edx, 18, Xstate::None)),
    ("amx-bf16", Probe::new(CpuWord::Leaf7Edx, 22, Xstate::Amx)),
    (
        "avx512fp16",
        Probe::new(CpuWord::Leaf7Edx, 23, Xstate::Avx512),
    ),
    ("amx-tile", Probe::new(CpuWord::Leaf7Edx, 24, Xstate::Amx)),
    ("amx-int8", Probe::new(CpuWord::Leaf7Edx, 25, Xstate::Amx)),
    // ---- leaf 7, subleaf 1 --------------------------------------------------
    ("sha512", Probe::new(CpuWord::Leaf71Eax, 0, Xstate::None)),
    ("sm3", Probe::new(CpuWord::Leaf71Eax, 1, Xstate::None)),
    ("sm4", Probe::new(CpuWord::Leaf71Eax, 2, Xstate::None)),
    ("raoint", Probe::new(CpuWord::Leaf71Eax, 3, Xstate::None)),
    ("avxvnni", Probe::new(CpuWord::Leaf71Eax, 4, Xstate::Vex)),
    (
        "avx512bf16",
        Probe::new(CpuWord::Leaf71Eax, 5, Xstate::Avx512),
    ),
    ("cmpccxadd", Probe::new(CpuWord::Leaf71Eax, 7, Xstate::None)),
    ("amx-fp16", Probe::new(CpuWord::Leaf71Eax, 21, Xstate::Amx)),
    ("hreset", Probe::new(CpuWord::Leaf71Eax, 22, Xstate::None)),
    ("avxifma", Probe::new(CpuWord::Leaf71Eax, 23, Xstate::Vex)),
    (
        "avxvnniint8",
        Probe::new(CpuWord::Leaf71Edx, 4, Xstate::Vex),
    ),
    (
        "avxneconvert",
        Probe::new(CpuWord::Leaf71Edx, 5, Xstate::Vex),
    ),
    (
        "amx-complex",
        Probe::new(CpuWord::Leaf71Edx, 8, Xstate::Amx),
    ),
    (
        "avxvnniint16",
        Probe::new(CpuWord::Leaf71Edx, 10, Xstate::Vex),
    ),
    (
        "prefetchi",
        Probe::new(CpuWord::Leaf71Edx, 14, Xstate::None),
    ),
    ("usermsr", Probe::new(CpuWord::Leaf71Edx, 15, Xstate::None)),
    ("movrs", Probe::new(CpuWord::Leaf71Eax, 31, Xstate::None)),
    // ---- leaf 0xD, subleaf 1 ------------------------------------------------
    ("xsaveopt", Probe::new(CpuWord::LeafD1Eax, 0, Xstate::Vex)),
    ("xsavec", Probe::new(CpuWord::LeafD1Eax, 1, Xstate::Vex)),
    ("xsaves", Probe::new(CpuWord::LeafD1Eax, 3, Xstate::Vex)),
    // ---- extended leaf 0x80000001 ------------------------------------------
    ("lahf-lm", Probe::new(CpuWord::Ext1Ecx, 0, Xstate::None)),
    ("lzcnt", Probe::new(CpuWord::Ext1Ecx, 5, Xstate::None)),
    ("sse4a", Probe::new(CpuWord::Ext1Ecx, 6, Xstate::None)),
    ("prfchw", Probe::new(CpuWord::Ext1Ecx, 8, Xstate::None)),
    ("xop", Probe::new(CpuWord::Ext1Ecx, 11, Xstate::Vex)),
    ("lwp", Probe::new(CpuWord::Ext1Ecx, 15, Xstate::None)),
    ("fma4", Probe::new(CpuWord::Ext1Ecx, 16, Xstate::Vex)),
    ("tbm", Probe::new(CpuWord::Ext1Ecx, 21, Xstate::Vex)),
    ("mwaitx", Probe::new(CpuWord::Ext1Ecx, 29, Xstate::None)),
    ("lm", Probe::new(CpuWord::Ext1Edx, 29, Xstate::None)),
    ("3dnow", Probe::new(CpuWord::Ext1Edx, 31, Xstate::None)),
    // ---- leaves 0x14 / 0x80000008 / 0x24 (AUD-CPU-1: the six names GCC 16.2
    // accepts that LCCC did not model before; rejecting them would have been a
    // gratuitous incompatibility, silently answering 0 the disability the
    // audit is about).
    ("ptwrite", Probe::new(CpuWord::Leaf14Ebx, 4, Xstate::None)),
    ("clzero", Probe::new(CpuWord::Ext8Ebx, 0, Xstate::None)),
    ("wbnoinvd", Probe::new(CpuWord::Ext8Ebx, 9, Xstate::None)),
    // AVX10: bit N of the synthetic word means "AVX10.N supported".
    ("avx10.1", Probe::new(CpuWord::Avx10Levels, 1, Xstate::None)),
    ("avx10.2", Probe::new(CpuWord::Avx10Levels, 2, Xstate::None)),
];

/// Alternative spellings accepted for the same feature.
///
/// `pclmulqdq`/`rdrand` are Clang/Intel spellings GCC rejects; accepting them
/// costs nothing and lets Intel-style dispatch code build unmodified.  The
/// remaining entries are the `_`-separated spellings used by `-m` flags and the
/// `__SSE4_2__`-style macros.
pub const NAME_ALIASES: &[(&str, &str)] = &[
    ("pclmulqdq", "pclmul"),
    ("rdrand", "rdrnd"),
    ("sse4_1", "sse4.1"),
    ("sse4_2", "sse4.2"),
    ("pni", "sse3"),
    ("abm", "lzcnt"),
    ("cmpxchg16b", "cx16"),
    ("lahf_lm", "lahf-lm"),
    ("amx_tile", "amx-tile"),
    ("amx_int8", "amx-int8"),
    ("amx_bf16", "amx-bf16"),
    ("amx_fp16", "amx-fp16"),
];

/// Symbol of the probe initialiser the driver appends to the translation unit.
pub const INIT_SYMBOL: &str = "__lccc_cpu_init";

/// Resolve a feature name (case-insensitively, with aliases) to its probe.
pub fn lookup(name: &str) -> Option<Probe> {
    let lower = name.to_ascii_lowercase();
    let canonical = NAME_ALIASES
        .iter()
        .find(|(alias, _)| *alias == lower)
        .map(|(_, canonical)| *canonical)
        .unwrap_or(lower.as_str());
    PROBE_TABLE
        .iter()
        .find(|(known, _)| *known == canonical)
        .map(|(_, probe)| *probe)
}

/// Whether `__builtin_cpu_supports` accepts this feature name.
pub fn is_known(name: &str) -> bool {
    lookup(name).is_some()
}

/// Whether `symbol` is one of the cached probe words.
///
/// Lowering uses this to find the first probe read in a function's entry block:
/// the initialising call is spliced immediately before it, which places the
/// call after every instruction that consumes an incoming ABI register
/// (parameter allocas, their `ParamRef`/`Store` pairs and the nested-function
/// static chain are all emitted at function entry) and before every read that
/// depends on it.
pub fn is_probe_word_symbol(symbol: &str) -> bool {
    CpuWord::ALL.iter().any(|word| word.symbol() == symbol)
}

/// Process-global "this TU used a runtime probe" flag.
///
/// Set by IR lowering (which has no driver handle) and consumed by the driver
/// right after code generation, when the generated assembly is still a `String`
/// it owns: the helper is appended exactly where the probe symbols it defines
/// are referenced.
static RUNTIME_PROBE_NEEDED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Record that the translation unit being lowered needs the probe helper.
pub fn mark_runtime_needed() {
    RUNTIME_PROBE_NEEDED.store(true, std::sync::atomic::Ordering::Release);
}

/// Clear the flag before lowering a new translation unit.
pub fn reset_runtime_needed() {
    RUNTIME_PROBE_NEEDED.store(false, std::sync::atomic::Ordering::Release);
}

/// Whether the translation unit just lowered referenced the probe helper.
pub fn take_runtime_needed() -> bool {
    RUNTIME_PROBE_NEEDED.swap(false, std::sync::atomic::Ordering::Release)
}

/// Whether the module now being lowered referenced the probe helper.
///
/// A peek: `mark_runtime_needed`/`reset_runtime_needed` own the flag's
/// lifetime.  Code generation uses it to decide whether the helper's symbols
/// belong to this translation unit (see [`defined_symbols`]).
pub fn runtime_needed() -> bool {
    RUNTIME_PROBE_NEEDED.load(std::sync::atomic::Ordering::Acquire)
}

/// Every symbol [`runtime_asm`] defines.
///
/// The helper is emitted *into the translation unit that references it*, so
/// these names are local definitions of that unit -- exactly like a `static`.
/// Backends consult this to address them directly (i686 PIC: `@GOTOFF`
/// through the PC-relative GOT base; x86-64: RIP-relative) instead of through
/// a GOT slot, which is both what GCC does for `__cpu_model` and what keeps
/// the probe free of link-time GOT entries.
pub fn defined_symbols() -> impl Iterator<Item = &'static str> {
    CpuWord::ALL
        .iter()
        .map(|word| word.symbol())
        .chain([READY_SYMBOL, INIT_SYMBOL])
}

/// Symbol of the helper's `ready` flag.
pub const READY_SYMBOL: &str = "__lccc_cpu_ready";

/// Data-access flavour of the generated helper.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mode {
    /// x86-64: every reference is RIP-relative, so the helper needs no base
    /// register and only the usual PC-relative relocation.
    X86_64,
    /// i686 without position independence: absolute references, exactly what
    /// the i686 backend emits for `GlobalAddr` in the same mode.
    I686Absolute,
    /// i686 with PIC/PIE: `@GOTOFF` through a GOT base derived in the
    /// prologue, mirroring what the i686 backend does for `GlobalAddr` when
    /// `CodegenOptions::pic`/`pie` is set.  The base is copied into `%esi`
    /// because CPUID clobbers `%ebx` and `%esi` is callee-saved under the
    /// i386 SysV ABI (so it survives every probe, and the helper restores it).
    I686GotRelative,
}

/// Addressing context of the generated helper.
struct Ctx {
    mode: Mode,
}

impl Ctx {
    /// A memory operand for a `.bss` word.
    fn word_operand(&self, word: CpuWord) -> String {
        self.symbol_operand(word.symbol())
    }

    /// A memory operand for an arbitrary helper symbol.
    fn symbol_operand(&self, symbol: &str) -> String {
        match self.mode {
            Mode::X86_64 => format!("{symbol}(%rip)"),
            Mode::I686Absolute => symbol.to_string(),
            Mode::I686GotRelative => format!("{symbol}@GOTOFF(%esi)"),
        }
    }

    /// `movl <reg>, <word>`.
    fn store(&self, word: CpuWord, reg: &str) -> String {
        format!("\tmovl\t{reg}, {}\n", self.word_operand(word))
    }

    /// `andl $imm, <word>`.
    fn and_word(&self, word: CpuWord, imm: u32) -> String {
        format!("\tandl\t${imm}, {}\n", self.word_operand(word))
    }

    /// The `ready` flag as a memory operand.
    fn ready_operand(&self) -> String {
        self.symbol_operand(READY_SYMBOL)
    }
}

/// The assembly text defining the probe words and [`INIT_SYMBOL`].
///
/// Only emitted for x86 targets; `__builtin_cpu_supports` never lowers to a
/// runtime probe on AArch64/RISC-V, so no caller exists to need this text
/// there.
///
/// Properties the generated code keeps:
///
/// * position independence — x86-64 uses RIP-relative references; i686 PIC
///   derives a GOT base from the return address of a local thunk (the standard
///   `R_386_GOTPC` idiom that `__x86.get_pc_thunk.bx` implements as a shared
///   stub) and i686 non-PIC uses absolute references; none of the three
///   introduces a text relocation,
/// * ABI-clean — only caller-saved registers are used, `%rbx`/`%ebx` (which
///   CPUID clobbers) and, in the i686 PIC flavour, `%esi` (which holds the GOT
///   base across CPUID) are saved and restored,
/// * idempotent and re-entrant — a second call, or one from another thread,
///   re-derives the same values; the words are written before the `ready` flag
///   and every field is a full 32-bit store, so a thread that observes
///   `ready == 1` observes consistent words,
/// * fail-closed — when CPUID reports nothing (or the OS disabled the xstate)
///   the words stay zero and every probe answers `0`,
/// * unwindable — with `-funwind-tables` the helper carries CFI that describes
///   the frame at every instruction, and without it no CFI directive (and thus
///   no `.eh_frame` entry) is emitted at all.
pub fn runtime_asm(target: ProbeTarget, pic: bool, emit_cfi: bool) -> String {
    let mode = match (target, pic) {
        (ProbeTarget::X86_64, _) => Mode::X86_64,
        (ProbeTarget::I686, false) => Mode::I686Absolute,
        (ProbeTarget::I686, true) => Mode::I686GotRelative,
    };
    let ctx = Ctx { mode };
    let mut out = String::with_capacity(4096);
    out.push_str("\t.bss\n");
    for word in CpuWord::ALL {
        out.push_str(&format!(
            "\t.local\t{sym}\n\t.p2align 2\n{sym}:\n\t.zero\t4\n",
            sym = word.symbol()
        ));
    }
    out.push_str(&format!(
        "\t.local\t{READY_SYMBOL}\n\t.p2align 2\n{READY_SYMBOL}:\n\t.zero\t4\n"
    ));
    out.push_str("\t.text\n\t.local\t");
    out.push_str(INIT_SYMBOL);
    out.push_str("\n\t.type\t");
    out.push_str(INIT_SYMBOL);
    out.push_str(", @function\n\t.p2align 4\n");
    out.push_str(INIT_SYMBOL);
    out.push_str(":\n");
    if emit_cfi {
        out.push_str("\t.cfi_startproc\n");
    }

    // Prologue: save what the helper clobbers beyond the call-clobbered set
    // (CPUID writes EAX/EBX/ECX/EDX; %ebx is callee-saved) and, under i686
    // PIC, derive the GOT base.
    match mode {
        Mode::X86_64 => {
            out.push_str("\tpushq\t%rbx\n");
            if emit_cfi {
                out.push_str("\t.cfi_def_cfa_offset 16\n\t.cfi_offset %rbx, -16\n");
            }
        }
        Mode::I686Absolute => {
            out.push_str("\tpushl\t%ebx\n");
            if emit_cfi {
                out.push_str("\t.cfi_def_cfa_offset 8\n\t.cfi_offset %ebx, -8\n");
            }
        }
        Mode::I686GotRelative => {
            out.push_str("\tpushl\t%esi\n\tpushl\t%ebx\n");
            if emit_cfi {
                out.push_str("\t.cfi_def_cfa_offset 8\n\t.cfi_offset %esi, -8\n");
                out.push_str("\t.cfi_def_cfa_offset 12\n\t.cfi_offset %ebx, -12\n");
            }
            out.push_str("\tcall\t.Llccc_cpu_got\n");
            out.push_str("\taddl\t$_GLOBAL_OFFSET_TABLE_, %ebx\n");
            // CPUID owns EBX, so the base lives in the callee-saved %esi.
            out.push_str("\tmovl\t%ebx, %esi\n");
        }
    }

    out.push_str(&format!("\tcmpl\t$0, {}\n", ctx.ready_operand()));
    out.push_str("\tjne\t.Llccc_cpu_done\n");

    // Leaf 1.  CPUID has no defined output for an unsupported leaf, so every
    // input is guarded by the maximum leaf the CPU advertises (leaf 0).  The
    // guards re-read leaf 0 rather than caching it: CPUID clobbers
    // EAX/EBX/ECX/EDX, the helper has no free callee-saved register to spare,
    // and this path runs once per program.
    out.push_str("\txorl\t%eax, %eax\n\tcpuid\n\tcmpl\t$1, %eax\n");
    out.push_str("\tjb\t.Llccc_cpu_no_leaf1\n");
    out.push_str("\tmovl\t$1, %eax\n\tcpuid\n");
    out.push_str(&ctx.store(CpuWord::Leaf1Edx, "%edx"));
    out.push_str(&ctx.store(CpuWord::Leaf1Ecx, "%ecx"));
    out.push_str(".Llccc_cpu_no_leaf1:\n");
    out.push_str("\txorl\t%eax, %eax\n\tcpuid\n\tcmpl\t$7, %eax\n");
    out.push_str("\tjb\t.Llccc_cpu_no_leaf7\n");
    out.push_str("\tmovl\t$7, %eax\n\txorl\t%ecx, %ecx\n\tcpuid\n");
    out.push_str(&ctx.store(CpuWord::Leaf7Ebx, "%ebx"));
    out.push_str(&ctx.store(CpuWord::Leaf7Ecx, "%ecx"));
    out.push_str(&ctx.store(CpuWord::Leaf7Edx, "%edx"));
    // EAX is the maximum subleaf of leaf 7; subleaf 1 needs its own guard.
    out.push_str("\tcmpl\t$1, %eax\n\tjb\t.Llccc_cpu_no_leaf7\n");
    out.push_str("\tmovl\t$7, %eax\n\tmovl\t$1, %ecx\n\tcpuid\n");
    out.push_str(&ctx.store(CpuWord::Leaf71Eax, "%eax"));
    out.push_str(&ctx.store(CpuWord::Leaf71Edx, "%edx"));
    out.push_str(".Llccc_cpu_no_leaf7:\n");
    // Leaf 0xD subleaf 1: the XSAVEOPT/XSAVEC/XSAVES management bits.
    out.push_str("\txorl\t%eax, %eax\n\tcpuid\n\tcmpl\t$0xd, %eax\n");
    out.push_str("\tjb\t.Llccc_cpu_no_leafd\n");
    out.push_str("\tmovl\t$0xd, %eax\n\tmovl\t$1, %ecx\n\tcpuid\n");
    out.push_str(&ctx.store(CpuWord::LeafD1Eax, "%eax"));
    out.push_str(".Llccc_cpu_no_leafd:\n");
    // Leaf 0x14 subleaf 0: PTWRITE (EBX bit 4).
    out.push_str("\txorl\t%eax, %eax\n\tcpuid\n\tcmpl\t$0x14, %eax\n");
    out.push_str("\tjb\t.Llccc_cpu_no_leaf14\n");
    out.push_str("\tmovl\t$0x14, %eax\n\txorl\t%ecx, %ecx\n\tcpuid\n");
    out.push_str(&ctx.store(CpuWord::Leaf14Ebx, "%ebx"));
    out.push_str(".Llccc_cpu_no_leaf14:\n");
    // Leaf 0x24 subleaf 0: the AVX10 version byte.  LCCC caches *levels*, not
    // the raw version, so `avx10.N` stays an ordinary bit test.
    out.push_str("\txorl\t%eax, %eax\n\tcpuid\n\tcmpl\t$0x24, %eax\n");
    out.push_str("\tjb\t.Llccc_cpu_no_avx10\n");
    out.push_str("\tmovl\t$0x24, %eax\n\txorl\t%ecx, %ecx\n\tcpuid\n");
    // The version leaf is only meaningful when the CPU advertises the AVX10
    // ISA extension itself (CPUID.(7,1):EDX[19]) — the same guard compiler-rt
    // and libgcc use.
    out.push_str("\ttestl\t$0x00080000, ");
    out.push_str(&ctx.word_operand(CpuWord::Leaf71Edx));
    out.push_str("\n\tje\t.Llccc_cpu_no_avx10\n");
    out.push_str("\tandl\t$0xff, %ebx\n");
    out.push_str("\tcmpl\t$1, %ebx\n\tjb\t.Llccc_cpu_no_avx10\n");
    out.push_str(&format!(
        "\torl\t$2, {}\n",
        ctx.word_operand(CpuWord::Avx10Levels)
    ));
    out.push_str("\tcmpl\t$2, %ebx\n\tjb\t.Llccc_cpu_no_avx10\n");
    out.push_str(&format!(
        "\torl\t$4, {}\n",
        ctx.word_operand(CpuWord::Avx10Levels)
    ));
    out.push_str(".Llccc_cpu_no_avx10:\n");
    // Extended leaves: leaf 0x80000000 returns the maximum extended leaf.
    out.push_str("\tmovl\t$0x80000000, %eax\n\tcpuid\n\tcmpl\t$0x80000001, %eax\n");
    out.push_str("\tjb\t.Llccc_cpu_no_ext\n");
    out.push_str("\tmovl\t$0x80000001, %eax\n\tcpuid\n");
    out.push_str(&ctx.store(CpuWord::Ext1Ecx, "%ecx"));
    out.push_str(&ctx.store(CpuWord::Ext1Edx, "%edx"));
    out.push_str(".Llccc_cpu_no_ext:\n");
    // Extended leaf 0x80000008: CLZERO (EBX bit 0) and WBNOINVD (bit 9).
    out.push_str("\tmovl\t$0x80000000, %eax\n\tcpuid\n\tcmpl\t$0x80000008, %eax\n");
    out.push_str("\tjb\t.Llccc_cpu_no_ext8\n");
    out.push_str("\tmovl\t$0x80000008, %eax\n\tcpuid\n");
    out.push_str(&ctx.store(CpuWord::Ext8Ebx, "%ebx"));
    out.push_str(".Llccc_cpu_no_ext8:\n");

    // XSTATE: OSXSAVE gates the XGETBV instruction itself.
    out.push_str("\ttestl\t$0x08000000, ");
    out.push_str(&ctx.word_operand(CpuWord::Leaf1Ecx));
    out.push_str("\n\tje\t.Llccc_cpu_no_xstate\n");
    out.push_str("\txorl\t%ecx, %ecx\n\txgetbv\n");
    out.push_str(&ctx.store(CpuWord::Xcr0, "%eax"));
    out.push_str(".Llccc_cpu_no_xstate:\n");
    for state in [Xstate::Vex, Xstate::Avx512, Xstate::Amx] {
        out.push_str(&state_gate_asm(state, &ctx));
    }
    out.push_str(&format!("\tmovl\t$1, {}\n", ctx.ready_operand()));

    // Epilogue.  Both exits land here, and every push has a matching pop on
    // both paths, so the stack shape is the same whether or not the body ran.
    out.push_str(".Llccc_cpu_done:\n");
    match mode {
        Mode::X86_64 => {
            out.push_str("\tpopq\t%rbx\n");
            if emit_cfi {
                out.push_str("\t.cfi_def_cfa_offset 8\n\t.cfi_restore %rbx\n");
            }
        }
        Mode::I686Absolute => {
            out.push_str("\tpopl\t%ebx\n");
            if emit_cfi {
                out.push_str("\t.cfi_def_cfa_offset 4\n\t.cfi_restore %ebx\n");
            }
        }
        Mode::I686GotRelative => {
            out.push_str("\tpopl\t%ebx\n");
            if emit_cfi {
                out.push_str("\t.cfi_def_cfa_offset 8\n\t.cfi_restore %ebx\n");
            }
            out.push_str("\tpopl\t%esi\n");
            if emit_cfi {
                out.push_str("\t.cfi_def_cfa_offset 4\n\t.cfi_restore %esi\n");
            }
        }
    }
    out.push_str("\tret\n");
    if emit_cfi {
        out.push_str("\t.cfi_endproc\n");
    }
    if mode == Mode::I686GotRelative {
        // Local PC thunk: reads the return address without popping it, so the
        // caller's `ret` still finds its own return address on the stack.  The
        // `addl $_GLOBAL_OFFSET_TABLE_` above resolves through R_386_GOTPC
        // against *this* instruction's address, exactly like the shared
        // `__x86.get_pc_thunk.bx` stub the backend emits for PIC functions.
        out.push_str(".Llccc_cpu_got:\n\tmovl\t(%esp), %ebx\n\tret\n");
    }
    out
}

/// The `and`-masks that clear every feature bit whose xstate the OS did not
/// enable, per word, derived from [`PROBE_TABLE`] so the two cannot disagree.
fn cleared_masks(state: Xstate) -> Vec<(CpuWord, u32)> {
    let mut per_word: Vec<(CpuWord, u32)> = Vec::new();
    for (_, probe) in PROBE_TABLE {
        if probe.state != state {
            continue;
        }
        match per_word.iter_mut().find(|(w, _)| *w == probe.word) {
            Some((_, mask)) => *mask |= probe.bit_mask(),
            None => per_word.push((probe.word, probe.bit_mask())),
        }
    }
    per_word
}

/// The xstate gate: when the required bits are missing from `XGETBV(0)`,
/// clear the corresponding feature bits so no probe can report them.
fn state_gate_asm(state: Xstate, ctx: &Ctx) -> String {
    let masks = cleared_masks(state);
    if masks.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    out.push_str("\tmovl\t");
    out.push_str(&ctx.word_operand(CpuWord::Xcr0));
    out.push_str(", %eax\n");
    out.push_str(&format!("\tandl\t${}, %eax\n", state.mask()));
    out.push_str(&format!("\tcmpl\t${}, %eax\n", state.mask()));
    out.push_str(&format!("\tje\t.Llccc_cpu_xstate_{}\n", state.label()));
    for (word, mask) in masks {
        out.push_str(&ctx.and_word(word, !mask));
    }
    out.push_str(&format!(".Llccc_cpu_xstate_{}:\n", state.label()));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::isa_features::FEATURE_TABLE;

    #[test]
    fn every_static_feature_name_has_a_runtime_probe() {
        // The static fold answers 1 for a name only if the effective target
        // guarantees it; a name outside this table would be rejected as
        // invalid even though the driver models it.  Keep the two tables in
        // sync in both directions.
        for name in FEATURE_TABLE {
            assert!(
                lookup(name).is_some(),
                "static feature {name} has no runtime probe entry"
            );
        }
    }

    #[test]
    fn lookups_and_aliases() {
        assert_eq!(lookup("SSE4.2").map(|p| p.bit), Some(20));
        assert_eq!(lookup("pclmulqdq").map(|p| p.word), Some(CpuWord::Leaf1Ecx));
        assert_eq!(lookup("rdrand").map(|p| p.bit), Some(30));
        assert_eq!(lookup("sse4_1"), lookup("sse4.1"));
        assert_eq!(lookup("amx_tile"), lookup("amx-tile"));
        assert_eq!(lookup("abm"), lookup("lzcnt"));
        assert_eq!(lookup("cmpxchg16b"), lookup("cx16"));
        assert_eq!(lookup("no-such-feature"), None);
        assert_eq!(lookup(""), None);
        // The six names GCC 16.2 accepts that LCCC lacked (census).
        assert_eq!(
            lookup("clzero").map(|p| (p.word, p.bit)),
            Some((CpuWord::Ext8Ebx, 0))
        );
        assert_eq!(
            lookup("wbnoinvd").map(|p| (p.word, p.bit)),
            Some((CpuWord::Ext8Ebx, 9))
        );
        assert_eq!(
            lookup("ptwrite").map(|p| (p.word, p.bit)),
            Some((CpuWord::Leaf14Ebx, 4))
        );
        assert_eq!(
            lookup("movrs").map(|p| (p.word, p.bit)),
            Some((CpuWord::Leaf71Eax, 31))
        );
        assert_eq!(
            lookup("avx10.1").map(|p| (p.word, p.bit)),
            Some((CpuWord::Avx10Levels, 1))
        );
        assert_eq!(
            lookup("avx10.2").map(|p| (p.word, p.bit)),
            Some((CpuWord::Avx10Levels, 2))
        );
    }

    #[test]
    fn spot_check_word_bit_state() {
        // Every entry here is anchored in compiler-rt's cpu_model/x86.c and is
        // cross-checked against GCC's own runtime answers by
        // tests/regression/check_cpu_supports_runtime.sh.
        let avx2 = lookup("avx2").expect("avx2");
        assert_eq!(
            (avx2.word, avx2.bit, avx2.state),
            (CpuWord::Leaf7Ebx, 5, Xstate::Vex)
        );
        let sha = lookup("sha").expect("sha");
        assert_eq!(
            (sha.word, sha.bit, sha.state),
            (CpuWord::Leaf7Ebx, 29, Xstate::None)
        );
        let avx512f = lookup("avx512f").expect("avx512f");
        assert_eq!((avx512f.word, avx512f.bit), (CpuWord::Leaf7Ebx, 16));
        assert_eq!(avx512f.state, Xstate::Avx512);
        let amx = lookup("amx-tile").expect("amx-tile");
        assert_eq!(
            (amx.word, amx.bit, amx.state),
            (CpuWord::Leaf7Edx, 24, Xstate::Amx)
        );
        assert_eq!(
            lookup("lzcnt").map(|p| (p.word, p.bit)),
            Some((CpuWord::Ext1Ecx, 5))
        );
        assert_eq!(lookup("sse2").map(|p| p.word), Some(CpuWord::Leaf1Edx));
        assert_eq!(
            lookup("xsaveopt").map(|p| (p.word, p.bit)),
            Some((CpuWord::LeafD1Eax, 0))
        );
        assert_eq!(
            lookup("sha512").map(|p| (p.word, p.bit)),
            Some((CpuWord::Leaf71Eax, 0))
        );
        assert_eq!(
            lookup("gfni").map(|p| (p.word, p.bit)),
            Some((CpuWord::Leaf7Ecx, 8))
        );
        assert_eq!(
            lookup("sse4a").map(|p| (p.word, p.bit)),
            Some((CpuWord::Ext1Ecx, 6))
        );
        assert_eq!(
            lookup("fma4").map(|p| (p.word, p.bit)),
            Some((CpuWord::Ext1Ecx, 16))
        );
        assert_eq!(
            lookup("3dnow").map(|p| (p.word, p.bit)),
            Some((CpuWord::Ext1Edx, 31))
        );
        assert_eq!(
            lookup("avx512er").map(|p| (p.word, p.bit)),
            Some((CpuWord::Leaf7Ebx, 27))
        );
    }

    #[test]
    fn avx10_levels_are_built_from_a_version_compare() {
        // bit 1 == AVX10.1, bit 2 == AVX10.2; the version byte is masked and
        // compared (never stored raw), and the AVX10 advertisement bit gates
        // the whole block.
        let asm = runtime_asm(ProbeTarget::X86_64, false, false);
        assert!(asm.contains("\tandl\t$0xff, %ebx\n"));
        assert!(asm.contains("\tcmpl\t$1, %ebx\n"));
        assert!(asm.contains("\tcmpl\t$2, %ebx\n"));
        let word = CpuWord::Avx10Levels.symbol();
        assert!(asm.contains(&format!("\torl\t$2, {word}(%rip)\n")));
        assert!(asm.contains(&format!("\torl\t$4, {word}(%rip)\n")));
        let adv = asm
            .find("\ttestl\t$0x00080000")
            .expect("AVX10 advertisement test");
        let first_or = asm
            .find(&format!("\torl\t$2, {word}"))
            .expect("bind AVX10.1");
        assert!(
            adv < first_or,
            "the level bits must be gated by the AVX10 bit"
        );
        assert!(lookup("avx10.1").is_some() && lookup("avx10.2").is_some());
    }

    #[test]
    fn probe_word_symbols_are_recognised() {
        assert!(is_probe_word_symbol(CpuWord::Leaf7Ebx.symbol()));
        assert!(is_probe_word_symbol("__lccc_cpu_wxcr0"));
        assert!(!is_probe_word_symbol(READY_SYMBOL));
        assert!(!is_probe_word_symbol(INIT_SYMBOL));
        assert!(!is_probe_word_symbol("memcpy"));
    }

    #[test]
    fn generated_assembly_covers_every_word_and_is_well_formed() {
        let asm = runtime_asm(ProbeTarget::X86_64, false, true);
        assert!(asm.contains(INIT_SYMBOL));
        for word in CpuWord::ALL {
            assert!(
                asm.contains(word.symbol()),
                "generated helper does not define/use {}",
                word.symbol()
            );
        }
        for state in [Xstate::Vex, Xstate::Avx512, Xstate::Amx] {
            assert!(
                !cleared_masks(state).is_empty(),
                "no feature uses {state:?}, the gate would be dead"
            );
            assert!(asm.contains(&format!(".Llccc_cpu_xstate_{}", state.label())));
        }
        // XGETBV may only run after OSXSAVE was tested.
        let osxsave = asm.find("\ttestl\t$0x08000000").expect("OSXSAVE test");
        let xgetbv = asm.find("\txgetbv").expect("xgetbv");
        assert!(
            osxsave < xgetbv,
            "xgetbv must be guarded by the OSXSAVE test"
        );
        // Leaf guards present for the leaves that need one.
        for guard in [
            "\tcmpl\t$0xd, %eax",
            "\tcmpl\t$7, %eax",
            "\tcmpl\t$0x14, %eax",
            "\tcmpl\t$0x24, %eax",
            "\tcmpl\t$0x80000001, %eax",
            "\tcmpl\t$0x80000008, %eax",
        ] {
            assert!(asm.contains(guard), "missing leaf guard {guard:?}");
        }
        // CFI is emitted only when the driver enables unwind tables, and it
        // must bracket the helper exactly once.
        assert_eq!(asm.matches(".cfi_startproc").count(), 1);
        assert_eq!(asm.matches(".cfi_endproc").count(), 1);
        assert!(asm.contains(".cfi_offset %rbx, -16"));
        assert!(asm.contains(".cfi_restore %rbx"));
        let no_cfi = runtime_asm(ProbeTarget::X86_64, false, false);
        assert!(!no_cfi.contains(".cfi_"));
        // Nothing in any flavour may touch a callee-saved register without
        // saving it, or address memory through a base the helper never sets.
        for (target, pic) in [
            (ProbeTarget::X86_64, false),
            (ProbeTarget::I686, false),
            (ProbeTarget::I686, true),
        ] {
            let variant = runtime_asm(target, pic, true);
            assert!(variant.contains("push"));
            assert!(variant.contains("pop"));
            assert!(!variant.contains("%rbp"));
            assert!(!variant.contains("%r12") && !variant.contains("%r13"));
        }
    }

    #[test]
    fn i686_flavours_address_words_without_pc_tricks() {
        let abs = runtime_asm(ProbeTarget::I686, false, true);
        assert!(abs.contains("pushl\t%ebx"));
        assert!(abs.contains("\tmovl\t%edx, __lccc_cpu_w1edx\n"));
        assert!(!abs.contains("@GOT"));
        assert!(!abs.contains("_GLOBAL_OFFSET_TABLE_"));
        assert!(!abs.contains("%rip"));
        assert!(!abs.contains("%r8"));
        assert!(!abs.contains("%esi"), "non-PIC needs no GOT base register");

        let pic = runtime_asm(ProbeTarget::I686, true, true);
        assert!(pic.contains("__lccc_cpu_w1edx@GOTOFF(%esi)"));
        assert!(pic.contains("__lccc_cpu_ready@GOTOFF(%esi)"));
        assert!(pic.contains("_GLOBAL_OFFSET_TABLE_"));
        assert!(pic.contains("\tmovl\t%ebx, %esi\n"));
        assert!(pic.contains("pushl\t%esi"));
        assert!(pic.contains("popl\t%esi"));
        assert!(pic.contains(".Llccc_cpu_got:"));
        assert!(!pic.contains("%rip"));
        // Both flavours save and restore %ebx (CPUID clobbers it).
        for variant in [&abs, &pic] {
            assert!(variant.contains("pushl\t%ebx"));
            assert!(variant.contains("popl\t%ebx"));
        }
        assert!(runtime_asm(ProbeTarget::I686, false, false).starts_with("\t.bss\n"));
    }

    #[test]
    fn cleared_masks_match_the_table() {
        let vex = cleared_masks(Xstate::Vex);
        let (_, mask) = vex
            .iter()
            .find(|(w, _)| *w == CpuWord::Leaf1Ecx)
            .expect("leaf 1 ECX has VEX-gated features");
        // AVX (28), FMA (12), F16C (29), XSAVE (26).
        assert_eq!(*mask, (1 << 28) | (1 << 12) | (1 << 29) | (1 << 26));
        let (_, mask) = vex
            .iter()
            .find(|(w, _)| *w == CpuWord::LeafD1Eax)
            .expect("leaf 0xD has VEX-gated features");
        assert_eq!(*mask, (1 << 0) | (1 << 1) | (1 << 3));
    }

    #[test]
    fn probe_words_are_unique_symbols() {
        let mut seen = std::collections::HashSet::new();
        for word in CpuWord::ALL {
            assert!(
                seen.insert(word.symbol()),
                "duplicate symbol {}",
                word.symbol()
            );
        }
        assert!(seen.insert(READY_SYMBOL));
        assert!(seen.insert(INIT_SYMBOL));
    }
}
