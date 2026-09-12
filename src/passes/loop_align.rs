//! Hot-loop alignment: compute per-block `.p2align` directives for the final
//! block layout.
//!
//! This pass runs POST-optimization, POST-label-renumber and POST-PGO-layout,
//! immediately before codegen (`driver::pipeline`). At that point the block
//! order in `IrFunction::blocks` is the emission order, so a backward branch
//! in that order identifies a loop header exactly the way the emitted
//! assembly will see it.
//!
//! # Policy (oracle-grounded)
//!
//! The directives were derived from what the reference compilers actually
//! emit for hot loops (Compiler Explorer, GCC 16.2 / Clang 23.1 / ICX
//! latest, x86-64-v3, verified 2026-09-08):
//!
//! * GCC 16.2 aligns every VECTORIZED loop header to 32 bytes
//!   unconditionally (`.p2align 5` / `.p2align 4` / `.p2align 3` cascade —
//!   a plain 32-byte alignment), *even under `-fno-align-loops`*: the
//!   vectorizer owns that alignment. Clang aligns loop headers to 16; ICX
//!   uses `.p2align 4, 0x90`. lccc takes the 32-byte intent bounded at 15
//!   padding bytes with a 16-byte fallback (see `vector_loop_cascade`):
//!   the DSB win whenever the offset is within reach, at most half of
//!   GCC's worst-case padding.
//! * GCC 16.2 aligns scalar loop headers with a bounded cascade:
//!   `.p2align 4,,10` followed by `.p2align 3` — 16 bytes when at most 10
//!   padding bytes are needed, otherwise at least 8 bytes. This is the
//!   `ASM_OUTPUT_MAX_SKIP_ALIGN` shape from gcc/config/i386.
//! * Nobody aligns anything at -Os (GCC -Os emits zero `.p2align`).
//! * Under PGO, alignment is reserved for blocks the profile says are hot
//!   (`pgo::layout` records those); cold loops receive no padding.
//! * Two static-mode refinements save padding that cannot pay off: loops
//!   whose header compare proves at most 4 trips, and functions the
//!   frontend marked cold (a `.text.unlikely`-style section) — neither
//!   can amortize the padding bytes, so neither is padded. An explicit
//!   `-falign-loops=N` is still honored everywhere.
//!
//! lccc adopts exactly this split, with one deliberate improvement in
//! controllability over GCC: `-fno-align-loops` also suppresses the
//! 32-byte vector-loop alignment (GCC cannot turn that one off), so the
//! whole policy is A/B-testable and `-Os`-safe by construction.
//!
//! # Hot scalar loops (structural 32-byte tier)
//!
//! GCC 16.2 does not stop at the 16/8 scalar cascade for every scalar
//! loop: a *dynamic, innermost, non-trivial* loop is promoted above it.
//! Godbolt census 2026-09-12 (GCC 16.2, `-O1`/`-O2`/`-O3`, plain and
//! `-march=raptorlake`, directive filter disabled so whole `.p2align`
//! groups are visible — with cascades the FIRST, strongest directive
//! dominates, see `HotScalarTier::Align32`):
//!
//! * the sqlite-style varint digit loop (branchy multi-block body,
//!   dynamic trip) gets unconditional `.p2align 5` (32 bytes) plus the
//!   16-skip-10/8 tail;
//! * larger branchy innermost bodies (7+ insns with an inner `if`) get
//!   unconditional `.p2align 6` (64 bytes) under raptorlake tuning;
//! * outer loops of nests keep the 16/8 cascade;
//! * single-block micro-bodies (load+add+branch reduction loops) keep
//!   16/8;
//! * constant-trip loops (even `i < 1000`) keep 16/8, and trips ≤4 get
//!   nothing at all.
//!
//! lccc mirrors that split in two layers (`audit_tight_loop` plus the
//! integrated assembler's `.lccc_tight_loop` resolution): only a loop
//! that is INNERMOST in the natural-loop nest, emits CONTIGUOUSLY from
//! its header to its latch, contains no call/inline-asm/indirect
//! transfer, has at most one conditional branch before the latch, and
//! is not a provably ≤4-trip constant loop is even a candidate; the
//! assembler then measures the exact encoded span and promotes to an
//! unconditional 16/32/64-byte header only when the body fits in one
//! cache line (size bucket `ceil(log2(span))`, log2 3..=6). Outer loops,
//! larger-than-cacheline bodies, reducible tiny constant-trip loops,
//! cold sections and -Os are unaffected, so the padding cost that made
//! a blanket 32-byte flip net-neutral cannot accumulate on cold/outer
//! headers. The tier is A/B-testable through `CCC_LOOP_ALIGN_HOT` (see
//! `TightLoopMode`); Clang 23.1 and ICX, which align every loop to 16,
//! bound the conservative side.
//!
//! Alignment improves the front end on Raptor Lake because the DSB (uop
//! cache) tracks 32-byte windows inside 64-byte lines: a 32-byte aligned
//! hot-loop head always starts a fresh window, and a 16-byte aligned
//! ordinary loop head keeps the loop inside a small number of fetch
//! windows. The earlier measured caveat (gzip `longest_match`,
//! engineering ledger) was about aligning *all* scalar loops to 32
//! unconditionally; this policy promotes only the structural hot class.

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::ir::analysis::CfgAnalysis;
use crate::ir::instruction::{BasicBlock, Instruction, Operand, Terminator};
use crate::ir::reexports::IrFunction;
use crate::passes::loop_analysis;

/// One `.p2align` directive to emit immediately before a block label.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AlignDirective {
    /// log2 of the byte alignment (4 = 16 bytes, 5 = 32 bytes).
    pub log2: u8,
    /// GAS third operand: skip the alignment entirely when more than this
    /// many padding bytes would be needed.
    pub max_skip: Option<u32>,
}

/// Per-unit decisions, keyed by block label. Codegen consumes this directly
/// before emitting each label (it supersedes the raw PGO alignment map,
/// which now serves as the hotness input).
static BLOCK_DIRECTIVES: std::sync::LazyLock<
    std::sync::Mutex<FxHashMap<u32, Vec<AlignDirective>>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(FxHashMap::default()));

/// Replace the per-unit directive map (called once per unit by the driver).
pub fn record_directives(map: FxHashMap<u32, Vec<AlignDirective>>) {
    *BLOCK_DIRECTIVES.lock().unwrap() = map;
}

/// Loop headers that passed the structural tight-loop audit. The
/// integrated assembler measures the exact encoded span and strengthens
/// these with an unconditional size-bucketed `.p2align K`; `-S` and
/// external-GAS builds never populate it (the portable bounded cascade
/// is the strongest portable contract).
static TIGHT_HEADERS: std::sync::LazyLock<std::sync::Mutex<FxHashSet<u32>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(FxHashSet::default()));

pub(crate) fn record_tight_headers(set: FxHashSet<u32>) {
    *TIGHT_HEADERS.lock().unwrap() = set;
}

/// True when `label` is a structurally qualifying tight-loop header
/// whose final alignment the integrated assembler must resolve.
pub fn tight_header(label: u32) -> bool {
    TIGHT_HEADERS.lock().unwrap().contains(&label)
}

/// The directives recorded for `label`, if any. Consumed by codegen
/// immediately before the block label is emitted.
pub fn directives(label: u32) -> Option<Vec<AlignDirective>> {
    BLOCK_DIRECTIVES.lock().unwrap().get(&label).cloned()
}

/// True when any alignment directives are recorded (fast bail-out for
/// codegen on plain builds).
pub fn directives_active() -> bool {
    !BLOCK_DIRECTIVES.lock().unwrap().is_empty()
}

/// User control from `-falign-loops[=N[:M]]` / `-fno-align-loops`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlignControl {
    /// Not mentioned on the command line: follow the -O level policy.
    Auto,
    /// `-fno-align-*`: suppress this alignment class entirely.
    Off,
    /// `-falign-loops=N[:M]`: alignment bytes and optional max-skip bytes.
    Custom { align: u32, max_skip: Option<u32> },
}

/// Per-unit inputs the pass needs from the driver.
pub struct LoopAlignConfig {
    /// 0=-O0 … 3=-O3, 4=-Os, 5=-Oz (the driver's numeric spelling).
    pub opt_level: u32,
    /// -Os/-Oz: size optimization disables all code padding.
    pub size_opt: bool,
    /// A PGO profile is in use: alignment is gated on recorded hotness.
    pub pgo_active: bool,
    /// `-falign-loops` control.
    pub loops: AlignControl,
    /// `-falign-jumps` control (hot join points under PGO).
    pub jumps: AlignControl,
    /// The generated assembly goes to lccc's INTEGRATED assembler, which
    /// resolves the tight-loop alignment directives using the exact
    /// encoded body sizes during its layout fixed point. A `-S` compile
    /// (or the optional external-GAS toolchain) cannot express that
    /// resolution, so tight-loop markers are emitted only when true.
    pub integrated_assembler: bool,
}

impl LoopAlignConfig {
    /// The scalar-loop cascade: 16 bytes when at most 10 bytes of padding
    /// are needed, otherwise fall back to 8 bytes. Mirrors GCC's
    /// `ASM_OUTPUT_MAX_SKIP_ALIGN` output for x86 loop headers.
    fn scalar_loop_cascade() -> [AlignDirective; 2] {
        [
            AlignDirective {
                log2: 4,
                max_skip: Some(10),
            },
            AlignDirective {
                log2: 3,
                max_skip: None,
            },
        ]
    }

    /// The vector-loop cascade: try 32 bytes when at most 15 padding bytes
    /// are needed, otherwise 16 bytes unbounded (at most 15 more).
    ///
    /// GCC 16.2 pads every vectorized loop header to 32 bytes with no skip
    /// clause at all — worst case 31 bytes of one-shot NOP padding. ICX and
    /// Clang 23.1 both stop at 16 bytes. The bounded cascade takes GCC's
    /// 32-byte intent whenever the natural offset is within one 16-byte
    /// quantum of it (the common case after ordinary layout), and degrades
    /// to exactly the ICX/Clang 16-byte alignment otherwise, capping the
    /// code-size cost at 15 bytes instead of 31. On the DSB (32-byte uop
    /// windows inside 64-byte lines) the 32-byte tier gives a loop head a
    /// fresh window; the 16-byte fallback never shares a decoder fetch
    /// window with the preheader's tail.
    fn vector_loop_cascade() -> [AlignDirective; 2] {
        [
            AlignDirective {
                log2: 5,
                max_skip: Some(15),
            },
            // Padding to a 16-byte boundary never exceeds 15 bytes, so a
            // max-skip clause here could never fire; emit the plain form.
            AlignDirective {
                log2: 4,
                max_skip: None,
            },
        ]
    }
}

/// Tight-loop alignment policy for a scalar loop header.
///
/// This is lccc's port of GCC's two-part loop alignment machinery:
///
/// 1. `compute_alignments` (gcc/final.cc) grants the ordinary loop
///    cascade (`.p2align 4,,10` / `.p2align 3` on x86-64) to a loop
///    header only when its statically *estimated* back-edge count
///    exceeds its fall-through entry count by
///    `-param=align-loop-iterations` (default **4**): a loop the
///    optimizer cannot prove to run at least ~5 iterations is not
///    worth padding. lccc's existing visible-constant-trip exclusion
///    (`bound <= 4`) is the conservative static counterpart.
/// 2. The `align_tight_loops` pass (gcc/config/i386/i386-features.cc,
///    `ix86_align_loops`; enabled by `X86_TUNE_ALIGN_TIGHT_LOOPS`,
///    which is on for every modern x86 tuning except Zhaoxin,
///    Cascade/Skylake-AVX512 and Atom — including the generic default
///    in GCC 14/16) promotes a loop header to an **unconditional**
///    `.p2align K` where `K = ceil(log2(encoded loop body bytes))`,
///    clamped to one cache line (log2 3..=6, i.e. 8..=64 bytes), when
///    all of the following hold:
///      * the loop is innermost and laid out **contiguously** (every
///        emitted block from header to the latch belongs to this
///        loop);
///      * the latch's terminator branches back to the header;
///      * the body contains neither a CALL nor inline asm (their sizes
///        and effects are unknown; GCC rejects both outright);
///      * before the latch there is at most ONE conditional branch and
///        no unconditional branch (an inner dense control-flow nest
///        defeats the "one fetch window" premise);
///      * the minimum encoded body size is ≤ `prefetch_block`,
///        i.e. 64 bytes for modern Intel/AMD cost tables.
///
/// The unconditional directive is emitted *before* the ordinary
/// cascade; GAS applies the strongest unconditional alignment (a
/// leading `.p2align 5` dominates the later `.p2align 4,,10` /
/// `.p2align 3` — verified empirically with GNU as, 2026-09-12).
///
/// Net effect, matching GCC exactly by encoded body bucket:
/// 17–32 bytes → `.p2align 5`, 33–64 bytes → `.p2align 6`, ≤16 bytes →
/// plain `.p2align 4` (stronger than the bounded cascade alone), larger
/// or disqualified loops keep the 16/8 cascade. This is precisely the
/// selective 32/64-byte class that blanket-aligning every scalar loop
/// misses: arith_loop (a >112-byte straight-line multiply body),
/// bitops, outer loops and CALL/asm loops stay on 16/8, while the
/// sqlite varint digit loops (17–32 B) land on 32 and small branchy
/// kernels (33–64 B) land on 64.
///
/// The size measurement itself runs in the INTEGRATED assembler. The IR
/// pass runs before the x86 encoder, where encoded lengths do not exist;
/// a per-IR-opcode size estimate census-checked against lccc's own
/// objects mis-buckets ~25% of loop bodies (memory folding, register
/// coalescing and branch shortening make the mapping context
/// dependent). Instead, a structurally qualifying header receives the
/// `.lccc_tight_loop LABEL` pseudo directive (see
/// `x86::assembler::parser::AsmItem::TightLoopAlign`) which the
/// integrated assembler resolves during its branch-relaxation fixed
/// point: with all encoded lengths and label offsets final, it measures
/// the exact span from the header to its first backward branch and
/// inserts the unconditional `.p2align K` with
/// `K = ceil(log2(span))` clamped to log2 3..=6, refusing bodies larger
/// than one cache line — the exact decision GCC's RTL pass makes,
/// evaluated against lccc's real bytes. The ordinary bounded cascade is
/// emitted after the marker, so the result is byte-equivalent to the
/// `.p2align K / .p2align 4,,10 / .p2align 3` group GCC emits (the first
/// unconditional directive dominates the rest under GAS). `-S` output
/// and the optional external-GAS toolchain cannot resolve the marker,
/// so they keep the portable bounded cascade only (`integrated_assembler`
/// gate). `scripts/align_decision_census.py` compares the achieved
/// binary alignment of loop headers against GCC/Clang objects.
///
/// `CCC_LOOP_ALIGN_HOT` selects the policy for A/B measurements:
/// `gcc` (default — exact size buckets via the assembler), `off`
/// (cascade only), `5`/`6` (force a fixed unconditional tier on every
/// passing loop — resolved at IR level, works under `-S` too), or
/// `5skip` (bounded 32 with a 16 fallback).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TightLoopMode {
    /// Disable the tight-loop pass; every scalar loop gets 16/8.
    Off,
    /// GCC-faithful size bucket rule (default).
    Gcc,
    /// Force unconditional 32 for every passing loop (A/B ladder).
    Force32,
    /// Force unconditional 64 for every passing loop (A/B ladder).
    Force64,
    /// Bounded 32 (at most 15 NOP bytes, else 16) for every passing loop.
    Bounded32,
}

impl TightLoopMode {
    fn from_env() -> Self {
        match std::env::var("CCC_LOOP_ALIGN_HOT")
            .unwrap_or_else(|_| "gcc".to_string())
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "gcc" | "" | "1" | "on" => TightLoopMode::Gcc,
            "off" | "none" | "0" => TightLoopMode::Off,
            "5" | "32" | "5u" | "force5" => TightLoopMode::Force32,
            "6" | "64" | "6u" | "force6" => TightLoopMode::Force64,
            "5skip" | "32bounded" | "5bounded" => TightLoopMode::Bounded32,
            other => {
                // Fail closed: an unknown knob must never pad more than
                // the size-bucketed policy.
                eprintln!(
                    "lccc: warning: unknown CCC_LOOP_ALIGN_HOT={other:?}, \
                     falling back to the GCC-size-bucketed tight-loop policy"
                );
                TightLoopMode::Gcc
            }
        }
    }

    /// Concrete unconditional directives for the FORCED A/B ladders;
    /// `None` for `Gcc`/`Off`, whose tight decisions are structural
    /// (`Gcc` resolves the tier in the assembler; `Off` never promotes).
    fn forced_directives(self) -> Option<Vec<AlignDirective>> {
        let head = match self {
            TightLoopMode::Off | TightLoopMode::Gcc => return None,
            TightLoopMode::Force32 => AlignDirective {
                log2: 5,
                max_skip: None,
            },
            TightLoopMode::Force64 => AlignDirective {
                log2: 6,
                max_skip: None,
            },
            TightLoopMode::Bounded32 => {
                return Some(LoopAlignConfig::vector_loop_cascade().to_vec());
            }
        };
        let mut v = vec![head];
        v.extend_from_slice(&LoopAlignConfig::scalar_loop_cascade());
        Some(v)
    }
}

/// GCC `-param=align-loop-iterations` default: loops estimated at ≤4
/// trips receive no alignment at all.
const MIN_TRIP_FOR_LOOP_ALIGN: i64 = 4;
/// Modern x86 instruction-cache line / `prefetch_block` budget; the
/// integrated assembler refuses to strengthen bodies larger than this.
/// Canonical home for the policy numbers — the ELF writer imports these
/// rather than mirroring them, so the structural audit and the assembler
/// decision can never drift apart.
pub(crate) const TIGHT_LOOP_MAX_BYTES: u64 = 64;
pub(crate) const TIGHT_LOOP_MIN_LOG2: u32 = 3;
pub(crate) const TIGHT_LOOP_MAX_LOG2: u32 = 6;

/// Size bucket for a tight-loop body whose exact encoded span is
/// `span_bytes`: `ceil(log2(span))` clamped to 8..=64, or `None` when the
/// body does not fit one instruction-cache line (or is empty). This is
/// GCC's `GEN_INT(ceil_log2(size))` decision evaluated against lccc's
/// exact assembler-measured bytes instead of RTL minimum-size estimates.
/// Shared by the structural pass and the ELF writer so the bucket table
/// has exactly one implementation.
pub(crate) fn tight_bucket_log2(span_bytes: u64) -> Option<u32> {
    if !(1..=TIGHT_LOOP_MAX_BYTES).contains(&span_bytes) {
        return None;
    }
    let raw = 64 - (span_bytes - 1).leading_zeros();
    Some(raw.clamp(TIGHT_LOOP_MIN_LOG2, TIGHT_LOOP_MAX_LOG2))
}

/// Compute and record the alignment directives for every function in the
/// module. Pure analysis: the IR is not modified, and the recorded labels
/// are the final (post-renumber, post-PGO-layout) block labels.
pub fn align_module(module: &crate::ir::module::IrModule, cfg: &LoopAlignConfig) {
    let mut map: FxHashMap<u32, Vec<AlignDirective>> = FxHashMap::default();
    let mut tight: FxHashSet<u32> = FxHashSet::default();

    if std::env::var("CCC_NO_LOOP_ALIGN").is_ok() {
        record_directives(map);
        record_tight_headers(tight);
        return;
    }

    // -O0 runs no optimizations: aligning unoptimized spill-heavy loops
    // only bloats the binary. -Os/-Oz forbid padding by policy (GCC -Os
    // emits no .p2align at all).
    let loops_on = match cfg.loops {
        AlignControl::Off => false,
        AlignControl::Custom { .. } => true,
        AlignControl::Auto => cfg.opt_level >= 1 && cfg.opt_level <= 3 && !cfg.size_opt,
    };
    let joins_on = match cfg.jumps {
        AlignControl::Off => false,
        AlignControl::Custom { .. } => true,
        AlignControl::Auto => cfg.opt_level >= 1 && cfg.opt_level <= 3 && !cfg.size_opt,
    };

    // PGO hotness map (recorded by pgo::layout for hot loop headers and
    // hot join points). Empty when no profile is in use — then every loop
    // header qualifies (static mode cannot know hotness, and GCC likewise
    // aligns all loops at -O2+ without a profile).
    let pgo_map: FxHashSet<u32> = if cfg.pgo_active {
        crate::pgo::block_align_keys()
    } else {
        FxHashSet::default()
    };

    for func in &module.functions {
        if func.is_declaration || func.blocks.len() < 2 {
            continue;
        }
        align_function(
            func, cfg, loops_on, joins_on, &pgo_map, &mut map, &mut tight,
        );
    }

    record_directives(map);
    record_tight_headers(tight);
}

fn align_function(
    func: &IrFunction,
    cfg: &LoopAlignConfig,
    loops_on: bool,
    joins_on: bool,
    pgo_map: &FxHashSet<u32>,
    map: &mut FxHashMap<u32, Vec<AlignDirective>>,
    tight: &mut FxHashSet<u32>,
) {
    // Cold functions (`__attribute__((cold))`, which the frontend lowers to
    // a `.text.unlikely`-style section) are never hot by construction:
    // aligning their loops pads the cold path for a hot benefit that can
    // never materialize.
    if func
        .section
        .as_deref()
        .is_some_and(|s| s.contains("unlikely"))
    {
        return;
    }
    // Natural loops on the final CFG: robust to layout order, and the loop
    // BODY set is needed to classify vector loops.
    let cfg_analysis = crate::ir::analysis::CfgAnalysis::build(func);
    let raw_loops = loop_analysis::find_natural_loops(
        cfg_analysis.num_blocks,
        &cfg_analysis.preds,
        &cfg_analysis.succs,
        &cfg_analysis.idom,
    );
    if raw_loops.is_empty() && !joins_on {
        return;
    }
    let loops = loop_analysis::merge_loops_by_header(raw_loops);
    // Natural-loop nesting: the tight-loop tier is reserved for
    // INNERMOST contiguous loops (see `audit_tight_loop`).
    let nest = loop_analysis::LoopNest::analyze(cfg_analysis.num_blocks, &loops);
    let tight_mode = TightLoopMode::from_env();

    // Backedge targets in FINAL BLOCK ORDER. The natural-loop header and
    // the backward-branch target coincide for any sane layout (the header
    // dominates the latch and the layout keeps loop bodies contiguous),
    // and this is exactly the label the emitted assembly will branch back
    // to. Using block order (not just dominators) keeps the decision
    // consistent with what the assembler will pad.
    let pos: FxHashMap<u32, usize> = func
        .blocks
        .iter()
        .enumerate()
        .map(|(i, b)| (b.label.0, i))
        .collect();
    let mut backedge_targets: FxHashSet<u32> = FxHashSet::default();
    for block in &func.blocks {
        let pp = *pos.get(&block.label.0).unwrap_or(&usize::MAX);
        let mut mark = |t: u32| {
            if let Some(&tl) = pos.get(&t) {
                if pp > tl {
                    backedge_targets.insert(t);
                }
            }
        };
        match &block.terminator {
            Terminator::Branch(x) => mark(x.0),
            Terminator::CondBranch {
                true_label,
                false_label,
                ..
            } => {
                mark(true_label.0);
                mark(false_label.0);
            }
            Terminator::Switch { cases, default, .. } => {
                for (_, x) in cases {
                    mark(x.0);
                }
                mark(default.0);
            }
            Terminator::IndirectBranch {
                possible_targets, ..
            } => {
                for x in possible_targets {
                    mark(x.0);
                }
            }
            _ => {}
        }
    }

    let entry_label = func.blocks.first().map(|b| b.label.0);

    // Loop headers: one directive list per header label.
    if loops_on {
        for (loop_idx, lp) in loops.iter().enumerate() {
            let header_label = func.blocks[lp.header].label.0;
            if Some(header_label) == entry_label {
                // Never pad before the function entry — function alignment
                // owns that position.
                continue;
            }
            // In PGO builds only profile-hot loops are padded; without a
            // profile every loop qualifies (GCC's static -O2 behavior).
            if cfg.pgo_active && !pgo_map.contains(&header_label) {
                continue;
            }
            // Constant tiny-trip loops stay unaligned (GCC and Clang both
            // leave `for (i = 0; i < 3; i++)` alone): padding executes once
            // but the loop body runs at most `bound` times, so the fetch
            // savings cannot pay back the padding. Only a *visible* constant
            // bound disqualifies — a loop with a dynamic bound is aligned
            // even when its runtime trip count happens to be small. A custom
            // `-falign-loops=N` is an explicit user contract and overrides
            // the exclusion.
            // Hoisted once: the visible constant trip bound both drives
            // the tiny-trip exclusion (≤4: no padding at all) and denies
            // hot-class promotion (GCC keeps every provably fixed-trip
            // loop on the ordinary 16/8 cascade).
            let trip_bound = if matches!(cfg.loops, AlignControl::Custom { .. }) {
                None
            } else {
                header_constant_trip_bound(&func.blocks[lp.header])
            };
            if trip_bound.is_some_and(|bound| bound <= 4) {
                continue;
            }
            let directives = match cfg.loops {
                AlignControl::Custom { align, max_skip } => custom_directives(align, max_skip),
                _ => {
                    let vector_body = lp.body.iter().any(|&bi| {
                        func.blocks
                            .get(bi)
                            .is_some_and(|b| block_has_vector_insn(b))
                    });
                    if vector_body {
                        LoopAlignConfig::vector_loop_cascade().to_vec()
                    } else {
                        if audit_tight_loop(func, lp, &nest, loop_idx, trip_bound).is_ok() {
                            if let Some(forced) = tight_mode.forced_directives() {
                                // A/B ladder: concrete unconditional
                                // directives chosen at IR level.
                                forced
                            } else if tight_mode == TightLoopMode::Gcc && cfg.integrated_assembler {
                                // Default policy: record the structural
                                // marker; the assembler resolves the
                                // size bucket against exact encoded
                                // lengths. The portable bounded cascade
                                // recorded here follows the marker.
                                tight.insert(header_label);
                                LoopAlignConfig::scalar_loop_cascade().to_vec()
                            } else {
                                LoopAlignConfig::scalar_loop_cascade().to_vec()
                            }
                        } else {
                            LoopAlignConfig::scalar_loop_cascade().to_vec()
                        }
                    }
                }
            };
            if !directives.is_empty() {
                map.insert(header_label, directives);
            }
        }
    }

    // Hot join points (PGO only): align the merge so the fall-in is
    // decode-friendly. This mirrors the previous PGO-only layout behavior
    // (16 bytes) but through the same flag machinery.
    if joins_on && cfg.pgo_active {
        let mut indeg: FxHashMap<u32, u32> = FxHashMap::default();
        for block in &func.blocks {
            match &block.terminator {
                Terminator::Branch(x) => *indeg.entry(x.0).or_insert(0) += 1,
                Terminator::CondBranch {
                    true_label,
                    false_label,
                    ..
                } => {
                    *indeg.entry(true_label.0).or_insert(0) += 1;
                    *indeg.entry(false_label.0).or_insert(0) += 1;
                }
                Terminator::Switch { cases, default, .. } => {
                    for (_, x) in cases {
                        *indeg.entry(x.0).or_insert(0) += 1;
                    }
                    *indeg.entry(default.0).or_insert(0) += 1;
                }
                Terminator::IndirectBranch {
                    possible_targets, ..
                } => {
                    for x in possible_targets {
                        *indeg.entry(x.0).or_insert(0) += 1;
                    }
                }
                _ => {}
            }
        }
        for block in &func.blocks {
            let l = block.label.0;
            if Some(l) == entry_label {
                continue;
            }
            // Join points only: multiple predecessors, NOT a loop header
            // (those were handled above), and profile-hot.
            if backedge_targets.contains(&l) || !pgo_map.contains(&l) {
                continue;
            }
            if indeg.get(&l).copied().unwrap_or(0) >= 2 {
                let directives = match cfg.jumps {
                    AlignControl::Custom { align, max_skip } => custom_directives(align, max_skip),
                    _ => LoopAlignConfig::scalar_loop_cascade().to_vec(),
                };
                // A join that is also a loop header already recorded its
                // (stronger) directives; never downgrade it.
                map.entry(l).or_insert(directives);
            }
        }
    }
}

/// Translate `-falign-loops=N[:M]` into a directive list: plain `N` pads
/// unconditionally (GCC `-falign-loops=32` -> `.p2align 5` with no skip
/// clause); `N:M` bounds the padding to M bytes.
fn custom_directives(align: u32, max_skip: Option<u32>) -> Vec<AlignDirective> {
    let log2 = align.max(1).checked_ilog2().unwrap_or(0) as u8;
    vec![AlignDirective { log2, max_skip }]
}

/// True when the block contains a SIMD intrinsic (vectorizer `Vec*` family
/// or the explicit-SIMD `P*256`/`Loadu256` family from <lccc/simd.h>).
fn block_has_vector_insn(block: &BasicBlock) -> bool {
    block
        .instructions
        .iter()
        .any(|inst| matches!(inst, Instruction::Intrinsic { op, .. } if op.is_vector_op()))
}

/// True when the IR instruction disqualifies a candidate tight loop.
/// Mirrors GCC's structural rejections: a body containing a real call,
/// an indirect call, inline asm, a trampoline init, or a nonlocal goto
/// cannot be measured safely (`ix86_min_insn_size` returns the −1
/// reject for calls/asm), so the structural audit denies the marker
/// rather than trusting an exact span the assembler cannot prove.
fn insn_is_tight_disqualified(inst: &Instruction) -> bool {
    matches!(
        inst,
        Instruction::Call { .. }
            | Instruction::CallIndirect { .. }
            | Instruction::InlineAsm { .. }
            | Instruction::InitTrampoline { .. }
            | Instruction::NonlocalGoto { .. }
    )
}

fn terminator_targets_header(term: &Terminator, header_label: u32) -> bool {
    match term {
        Terminator::Branch(x) => x.0 == header_label,
        Terminator::CondBranch {
            true_label,
            false_label,
            ..
        } => true_label.0 == header_label || false_label.0 == header_label,
        Terminator::Switch { cases, default, .. } => {
            default.0 == header_label || cases.iter().any(|(_, x)| x.0 == header_label)
        }
        Terminator::IndirectBranch {
            possible_targets, ..
        } => possible_targets.iter().any(|x| x.0 == header_label),
        _ => false,
    }
}

/// Audit one natural loop against the STRUCTURAL half of GCC's
/// `align_tight_loops` criteria (`ix86_align_loops`,
/// gcc/config/i386/i386-features.cc). The encoded-size half (body
/// ≤ one cache line, size-bucketed alignment) is resolved later by the
/// integrated assembler using exact encoded lengths.
///
/// Required:
/// 1. **Innermost** loop in the natural-loop nest (GCC's contiguous
///    layout walk meets the inner loop and rejects an outer one).
/// 2. **Static frequency gate**: the header exit must not prove ≤
///    `align-loop-iterations` (4) trips — same bound as the existing
///    tiny-trip exclusion.
/// 3. **Contiguous final layout**: every block from the header to the
///    latch is emitted consecutively, with no foreign block interleaved
///    and the header physically first.
/// 4. **The latch ends the body**: the last contiguous block branches
///    back to the header.
/// 5. **No disqualified instructions** in the body (real calls, inline
///    asm, trampolines, nonlocal gotos — sizes/effects unknown).
/// 6. **Branch budget**: before the latch there is at most one
///    conditional branch and no unconditional or indirect transfer
///    (the dense fetch-window premise).
///
/// The audit only ever suppresses/strengthens padding and cannot change
/// semantics. `constant_bound` is the header's visible trip bound
/// (hoisted by the caller); `None` means the trip count is unknown.
fn audit_tight_loop(
    func: &IrFunction,
    lp: &loop_analysis::NaturalLoop,
    nest: &loop_analysis::LoopNest,
    loop_idx: usize,
    constant_bound: Option<i64>,
) -> Result<(), &'static str> {
    let result = audit_tight_loop_inner(func, lp, nest, loop_idx, constant_bound);
    if std::env::var("CCC_DUMP_ALIGN").is_ok() {
        let header_label = func.blocks[lp.header].label.0;
        match result {
            Ok(()) => eprintln!(
                "loop_align: func={} header=L{header_label} tight=yes bound={constant_bound:?}",
                func.name
            ),
            Err(reason) => eprintln!(
                "loop_align: func={} header=L{header_label} tight=no ({reason})                  bound={constant_bound:?}",
                func.name
            ),
        }
    }
    result
}

fn audit_tight_loop_inner(
    func: &IrFunction,
    lp: &loop_analysis::NaturalLoop,
    nest: &loop_analysis::LoopNest,
    loop_idx: usize,
    constant_bound: Option<i64>,
) -> Result<(), &'static str> {
    if !nest.children[loop_idx].is_empty() {
        return Err("outer loop");
    }
    if constant_bound.is_some_and(|b| b <= MIN_TRIP_FOR_LOOP_ALIGN) {
        return Err("constant trip <= 4");
    }

    // Contiguous emission layout: one unbroken run of consecutive blocks
    // in final order, beginning at the header and ending at the latch.
    let label_pos: FxHashMap<u32, usize> = func
        .blocks
        .iter()
        .enumerate()
        .map(|(i, b)| (b.label.0, i))
        .collect();
    let mut positions: Vec<usize> = lp
        .body
        .iter()
        .filter_map(|&bi| {
            func.blocks
                .get(bi)
                .and_then(|b| label_pos.get(&b.label.0).copied())
        })
        .collect();
    positions.sort_unstable();
    positions.dedup();
    if positions.len() != lp.body.len() {
        return Err("unmapped body block");
    }
    for w in positions.windows(2) {
        if w[1] != w[0] + 1 {
            return Err("non-contiguous layout");
        }
    }
    let (first, last) = match (positions.first(), positions.last()) {
        (Some(&f), Some(&l)) => (f, l),
        _ => return Err("empty body"),
    };
    let header_label = func.blocks[lp.header].label.0;
    if first != *label_pos.get(&header_label).unwrap_or(&usize::MAX) {
        return Err("header not first in layout");
    }
    if !terminator_targets_header(&func.blocks[last].terminator, header_label) {
        return Err("last body block is not the latch");
    }

    let mut cond_before_latch = 0u32;
    for (k, &bi) in positions.iter().enumerate() {
        let is_latch = k == positions.len() - 1;
        let block = &func.blocks[bi];
        for inst in &block.instructions {
            if insn_is_tight_disqualified(inst) {
                return Err("call, inline asm or nonlocal transfer in body");
            }
        }
        if is_latch {
            continue;
        }
        match block.terminator {
            Terminator::CondBranch { .. } => {
                cond_before_latch += 1;
                if cond_before_latch >= 2 {
                    return Err(">=2 conditional branches before latch");
                }
            }
            Terminator::Branch(_)
            | Terminator::Switch { .. }
            | Terminator::IndirectBranch { .. } => {
                return Err("unconditional/indirect transfer before latch");
            }
            _ => {}
        }
    }
    Ok(())
}

/// Visible constant trip bound of a counted loop from its *header exit*
/// comparison: `Some(n)` when the compare feeding the header's conditional
/// branch proves at most `n` iterations — `iv < Const(n)`, `iv <=
/// Const(n)` (bound `n + 1`), `iv != Const(n)` over the overwhelmingly
/// common forward induction (exactly `n` trips, matching GCC/Clang, which
/// leave constant-trip-4-or-less `!=` loops unaligned), or the mirrored
/// `Const(n) > iv` / `Const(n) >= iv` / `Const(n) != iv` forms.
///
/// Comparisons that do not feed the header terminator are ignored — a stray
/// constant compare inside the header must not disqualify alignment. The
/// bound measured here is the *runtime* iteration count of THIS loop: a
/// vectorized main loop whose bound was scaled by the vector width keeps
/// the scaled bound, and a remainder loop divided down keeps its own, which
/// is exactly what alignment payback depends on.
///
/// Only a non-negative integer constant bound disqualifies: `iv < -5` from
/// an unknown start proves nothing (pad as usual). The exclusion only ever
/// suppresses padding — it can cost front-end cycles on a misjudged hot
/// loop, but it can never affect correctness.
fn header_constant_trip_bound(header: &BasicBlock) -> Option<i64> {
    use crate::ir::ops::IrCmpOp;
    use crate::ir::reexports::Operand;

    let exit_cond = match &header.terminator {
        Terminator::CondBranch { cond, .. } => match cond {
            Operand::Value(v) => Some(*v),
            _ => None,
        },
        _ => None,
    }?;
    for inst in &header.instructions {
        if let Instruction::Cmp {
            dest, op, lhs, rhs, ..
        } = inst
        {
            if *dest != exit_cond {
                continue;
            }
            let (limit, flipped) = match rhs {
                Operand::Const(c) => (c, false),
                // The mirrored `Const(n) > iv` / `Const(n) >= iv` /
                // `Const(n) != iv` forms bound the trip too.
                Operand::Value(_) => match lhs {
                    Operand::Const(c) => (c, true),
                    _ => continue,
                },
            };
            let limit = limit.to_i64()?;
            // A negative limit proves nothing from an unknown start (pad as
            // usual); the caller only excludes small bounds anyway.
            if limit < 0 {
                continue;
            }
            // Ordering exits bound the trip above; `<=`/`>=` run one past
            // the limit. Lower-bound-only shapes (`iv > n`, `iv >= n`,
            // `n <= iv`, `n < iv`) and non-ordering exits prove nothing.
            let bound = match op {
                IrCmpOp::Slt | IrCmpOp::Ult if !flipped => limit,
                IrCmpOp::Sgt | IrCmpOp::Ugt if flipped => limit,
                IrCmpOp::Sle | IrCmpOp::Ule if !flipped => limit.saturating_add(1),
                IrCmpOp::Sge | IrCmpOp::Uge if flipped => limit.saturating_add(1),
                IrCmpOp::Ne => limit,
                _ => continue,
            };
            return Some(bound);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::types::IrType;
    use crate::ir::instruction::{BlockId, Value};
    use crate::ir::ops::IrCmpOp;
    use crate::ir::reexports::IrConst;
    use crate::passes::loop_analysis::NaturalLoop;

    fn blk(label: u32, insns: Vec<Instruction>, term: Terminator) -> BasicBlock {
        BasicBlock {
            label: BlockId(label),
            instructions: insns,
            terminator: term,
            source_spans: Vec::new(),
        }
    }

    fn br(t: u32) -> Terminator {
        Terminator::Branch(BlockId(t))
    }

    fn cond_on(cond_v: u32, t: u32, f: u32) -> Terminator {
        Terminator::CondBranch {
            cond: Operand::Value(Value(cond_v)),
            true_label: BlockId(t),
            false_label: BlockId(f),
        }
    }

    fn ret() -> Terminator {
        Terminator::Return(Some(Operand::Const(IrConst::I32(0))))
    }

    fn cmp(dest: u32, op: IrCmpOp, lhs: Operand, rhs: Operand) -> Instruction {
        Instruction::Cmp {
            dest: Value(dest),
            op,
            lhs,
            rhs,
            ty: IrType::I32,
        }
    }

    fn iv(v: u32) -> Operand {
        Operand::Value(Value(v))
    }
    fn k(n: i64) -> Operand {
        Operand::Const(IrConst::I64(n))
    }

    fn loop_header_block(op: IrCmpOp, limit: Operand, flipped: bool) -> BasicBlock {
        // %9 feeds the terminator; the compare form is varied by the test.
        let c = if flipped {
            cmp(9, op, limit, iv(1))
        } else {
            cmp(9, op, iv(1), limit)
        };
        blk(0, vec![c], cond_on(9, 2, 1))
    }

    // ── header_constant_trip_bound: the tiny-trip gate's proof rules ──

    #[test]
    fn trip_bound_ordering_exits() {
        // iv < N        -> N
        let h = loop_header_block(IrCmpOp::Ult, k(1000), false);
        assert_eq!(header_constant_trip_bound(&h), Some(1000));
        let h = loop_header_block(IrCmpOp::Slt, k(1000), false);
        assert_eq!(header_constant_trip_bound(&h), Some(1000));
        // iv <= N       -> N + 1
        let h = loop_header_block(IrCmpOp::Ule, k(1000), false);
        assert_eq!(header_constant_trip_bound(&h), Some(1001));
        let h = loop_header_block(IrCmpOp::Sle, k(1000), false);
        assert_eq!(header_constant_trip_bound(&h), Some(1001));
        // Mirrored Const > iv / Const >= iv bound the trip too.
        let h = loop_header_block(IrCmpOp::Sgt, k(1000), true);
        assert_eq!(header_constant_trip_bound(&h), Some(1000));
        let h = loop_header_block(IrCmpOp::Sge, k(1000), true);
        assert_eq!(header_constant_trip_bound(&h), Some(1001));
        // iv != N       -> N
        let h = loop_header_block(IrCmpOp::Ne, k(1000), false);
        assert_eq!(header_constant_trip_bound(&h), Some(1000));
    }

    #[test]
    fn trip_bound_le_saturates_at_i64_max() {
        let h = loop_header_block(IrCmpOp::Sle, k(i64::MAX), false);
        assert_eq!(header_constant_trip_bound(&h), Some(i64::MAX));
    }

    #[test]
    fn trip_bound_proves_nothing_for_lower_bounds_and_equality() {
        // Lower-bound-only exits (iv > N, iv >= N and the mirrored
        // inverses) prove no upper trip count.
        for op in [IrCmpOp::Sgt, IrCmpOp::Sge, IrCmpOp::Ugt, IrCmpOp::Uge] {
            let h = loop_header_block(op, k(4), false);
            assert_eq!(header_constant_trip_bound(&h), None, "normal {op:?}");
        }
        for op in [IrCmpOp::Slt, IrCmpOp::Sle, IrCmpOp::Ult, IrCmpOp::Ule] {
            let h = loop_header_block(op, k(4), true);
            assert_eq!(header_constant_trip_bound(&h), None, "flipped {op:?}");
        }
        // Equality is not an ordering exit.
        let h = loop_header_block(IrCmpOp::Eq, k(4), false);
        assert_eq!(header_constant_trip_bound(&h), None);
    }

    #[test]
    fn trip_bound_negative_limit_and_unrelated_compares_prove_nothing() {
        // iv < -5 with an unknown start proves nothing.
        let h = loop_header_block(IrCmpOp::Slt, k(-5), false);
        assert_eq!(header_constant_trip_bound(&h), None);
        // The terminator feeds %9; a compare defining %8 on the same
        // block must not be mistaken for the exit condition.
        let mut h = loop_header_block(IrCmpOp::Ult, k(1000), false);
        h.instructions.insert(0, cmp(8, IrCmpOp::Ult, iv(1), k(2)));
        assert_eq!(header_constant_trip_bound(&h), Some(1000));
        // No conditional terminator at all -> nothing to prove.
        let h = blk(0, Vec::new(), ret());
        assert_eq!(header_constant_trip_bound(&h), None);
        // The compare feeding the terminator must be on THIS block.
        let h = blk(0, Vec::new(), cond_on(9, 2, 1));
        assert_eq!(header_constant_trip_bound(&h), None);
    }

    // ── exact-span size buckets (shared with the ELF writer) ──

    #[test]
    fn tight_bucket_table_matches_one_cacheline_goal() {
        assert_eq!(tight_bucket_log2(0), None);
        for size in 1u64..=8 {
            assert_eq!(tight_bucket_log2(size), Some(3), "size {size}");
        }
        for size in 9u64..=16 {
            assert_eq!(tight_bucket_log2(size), Some(4), "size {size}");
        }
        for size in 17u64..=32 {
            assert_eq!(tight_bucket_log2(size), Some(5), "size {size}");
        }
        for size in 33u64..=64 {
            assert_eq!(tight_bucket_log2(size), Some(6), "size {size}");
        }
        assert_eq!(tight_bucket_log2(65), None);
        assert_eq!(tight_bucket_log2(u64::MAX), None);
    }

    // ── structural tight-loop audit (GCC ix86_align_loops mirror) ──

    fn audit(func: &IrFunction, lp: &NaturalLoop, bound: Option<i64>) -> Result<(), &'static str> {
        let nest = loop_analysis::LoopNest::analyze(func.blocks.len(), std::slice::from_ref(lp));
        audit_tight_loop_inner(func, lp, &nest, 0, bound)
    }

    fn func_of(blocks: Vec<BasicBlock>) -> IrFunction {
        let mut f = IrFunction::new("t".into(), IrType::I32, Vec::new(), false);
        f.blocks = blocks;
        f
    }

    /// Canonical accepted loop: blocks 0 header, 1 latch (cond back to
    /// 0), block 2 exit. Single conditional branch before the latch.
    fn accepted_func() -> (IrFunction, NaturalLoop) {
        let f = func_of(vec![
            loop_header_block(IrCmpOp::Ult, k(1000), false),
            blk(1, vec![], cond_on(9, 0, 2)),
            blk(2, vec![], ret()),
        ]);
        let lp = NaturalLoop {
            header: 0,
            body: FxHashSet::from_iter([0, 1]),
        };
        (f, lp)
    }

    #[test]
    fn audit_accepts_contiguous_innermost_loop() {
        let (f, lp) = accepted_func();
        assert!(audit(&f, &lp, Some(1000)).is_ok());
        // Single-block counted loop (header is its own latch).
        let f = func_of(vec![
            blk(
                0,
                vec![cmp(9, IrCmpOp::Ult, iv(1), k(1000))],
                cond_on(9, 0, 1),
            ),
            blk(1, vec![], ret()),
        ]);
        let lp = NaturalLoop {
            header: 0,
            body: FxHashSet::from_iter([0]),
        };
        assert!(audit(&f, &lp, Some(1000)).is_ok());
    }

    #[test]
    fn audit_rejects_provably_tiny_trips() {
        let (f, lp) = accepted_func();
        assert_eq!(audit(&f, &lp, Some(4)).unwrap_err(), "constant trip <= 4");
        // An unknown trip count is aligned (GCC's static-guess behavior).
        assert!(audit(&f, &lp, None).is_ok());
    }

    #[test]
    fn audit_rejects_outer_loops() {
        // Outer {0,1,2} containing inner {1,2}; hierarchy built by
        // LoopNest::analyze from containment.
        let f = func_of(vec![
            blk(0, vec![], cond_on(1, 1, 3)),
            blk(
                1,
                vec![cmp(9, IrCmpOp::Ult, iv(1), k(1000))],
                cond_on(9, 1, 2),
            ),
            blk(2, vec![], cond_on(1, 0, 3)),
            blk(3, vec![], ret()),
        ]);
        let outer = NaturalLoop {
            header: 0,
            body: FxHashSet::from_iter([0, 1, 2]),
        };
        let inner = NaturalLoop {
            header: 1,
            body: FxHashSet::from_iter([1, 2]),
        };
        let nest = loop_analysis::LoopNest::analyze(f.blocks.len(), &[outer.clone(), inner]);
        let outer_idx = nest.loops.iter().position(|l| l.header == 0).unwrap();
        assert_eq!(
            audit_tight_loop_inner(&f, &nest.loops[outer_idx], &nest, outer_idx, None).unwrap_err(),
            "outer loop"
        );
    }

    #[test]
    fn audit_rejects_non_contiguous_and_misordered_layout() {
        // Foreign exit block interleaved between header and latch.
        let f = func_of(vec![
            loop_header_block(IrCmpOp::Ult, k(1000), false),
            blk(1, vec![], ret()),
            blk(2, vec![], cond_on(9, 0, 3)),
            blk(3, vec![], ret()),
        ]);
        let lp = NaturalLoop {
            header: 0,
            body: FxHashSet::from_iter([0, 2]),
        };
        assert_eq!(audit(&f, &lp, None).unwrap_err(), "non-contiguous layout");

        // Body block emitted BEFORE its own header.
        let f = func_of(vec![
            blk(5, vec![], cond_on(9, 3, 4)),
            blk(
                3,
                vec![cmp(9, IrCmpOp::Ult, iv(1), k(1000))],
                cond_on(9, 4, 3),
            ),
            blk(4, vec![], ret()),
        ]);
        let lp = NaturalLoop {
            header: 1,
            body: FxHashSet::from_iter([0, 1]),
        };
        assert_eq!(
            audit(&f, &lp, None).unwrap_err(),
            "header not first in layout"
        );
    }

    #[test]
    fn audit_rejects_transfer_budget_violations() {
        // Unconditional jump before the latch.
        let f = func_of(vec![
            loop_header_block(IrCmpOp::Ult, k(1000), false),
            blk(1, vec![], br(3)),
            blk(2, vec![], cond_on(9, 0, 3)),
            blk(3, vec![], ret()),
        ]);
        let lp = NaturalLoop {
            header: 0,
            body: FxHashSet::from_iter([0, 1, 2]),
        };
        assert_eq!(
            audit(&f, &lp, None).unwrap_err(),
            "unconditional/indirect transfer before latch"
        );

        // Two conditional branches before the latch.
        let f = func_of(vec![
            loop_header_block(IrCmpOp::Ult, k(1000), false),
            blk(1, vec![cmp(8, IrCmpOp::Ult, iv(2), k(7))], cond_on(8, 0, 2)),
            blk(2, vec![], cond_on(9, 0, 3)),
            blk(3, vec![], ret()),
        ]);
        let lp = NaturalLoop {
            header: 0,
            body: FxHashSet::from_iter([0, 1, 2]),
        };
        assert_eq!(
            audit(&f, &lp, None).unwrap_err(),
            ">=2 conditional branches before latch"
        );

        // The last contiguous block does not jump back to the header.
        let f = func_of(vec![
            loop_header_block(IrCmpOp::Ult, k(1000), false),
            blk(1, vec![], cond_on(9, 2, 2)),
            blk(2, vec![], ret()),
        ]);
        let lp = NaturalLoop {
            header: 0,
            body: FxHashSet::from_iter([0, 1]),
        };
        assert_eq!(
            audit(&f, &lp, None).unwrap_err(),
            "last body block is not the latch"
        );
    }

    #[test]
    fn audit_rejects_disqualified_body_instructions() {
        let bad = Instruction::NonlocalGoto {
            chain: Operand::Const(IrConst::I64(0)),
            up: 0,
            rbp_off: 0,
            rsp_off: 0,
            label: "Lx".into(),
        };
        let f = func_of(vec![
            blk(
                0,
                vec![cmp(9, IrCmpOp::Ult, iv(1), k(1000)), bad],
                cond_on(9, 0, 1),
            ),
            blk(1, vec![], ret()),
        ]);
        let lp = NaturalLoop {
            header: 0,
            body: FxHashSet::from_iter([0]),
        };
        assert_eq!(
            audit(&f, &lp, None).unwrap_err(),
            "call, inline asm or nonlocal transfer in body"
        );
    }

    // ── A/B ladder directive shapes ──

    #[test]
    fn forced_ladders_emit_unconditional_head() {
        let v = TightLoopMode::Force32.forced_directives().unwrap();
        assert_eq!(v[0].log2, 5);
        assert_eq!(v[0].max_skip, None);
        assert_eq!(v[1..], LoopAlignConfig::scalar_loop_cascade());

        let v = TightLoopMode::Force64.forced_directives().unwrap();
        assert_eq!(v[0].log2, 6);
        assert_eq!(v[0].max_skip, None);

        assert_eq!(TightLoopMode::Gcc.forced_directives(), None);
        assert_eq!(TightLoopMode::Off.forced_directives(), None);

        assert_eq!(
            TightLoopMode::Bounded32.forced_directives().unwrap(),
            LoopAlignConfig::vector_loop_cascade().to_vec()
        );
    }

    #[test]
    fn unknown_tight_mode_knob_fails_closed_to_gcc_policy() {
        // SAFETY: single-threaded env mutation within this test; the knob
        // is read nowhere else in the unit-test process.
        unsafe {
            std::env::set_var("CCC_LOOP_ALIGN_HOT", "garbage");
            assert_eq!(TightLoopMode::from_env(), TightLoopMode::Gcc);
            std::env::remove_var("CCC_LOOP_ALIGN_HOT");
            assert_eq!(TightLoopMode::from_env(), TightLoopMode::Gcc);
            std::env::set_var("CCC_LOOP_ALIGN_HOT", "off");
            assert_eq!(TightLoopMode::from_env(), TightLoopMode::Off);
            std::env::remove_var("CCC_LOOP_ALIGN_HOT");
        }
    }
}
