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
//!   uses `.p2align 4, 0x90`.
//! * GCC 16.2 aligns scalar loop headers with a bounded cascade:
//!   `.p2align 4,,10` followed by `.p2align 3` — 16 bytes when at most 10
//!   padding bytes are needed, otherwise at least 8 bytes. This is the
//!   `ASM_OUTPUT_MAX_SKIP_ALIGN` shape from gcc/config/i386.
//! * Nobody aligns anything at -Os (GCC -Os emits zero `.p2align`).
//! * Under PGO, alignment is reserved for blocks the profile says are hot
//!   (`pgo::layout` records those); cold loops receive no padding.
//!
//! lccc adopts exactly this split, with one deliberate improvement in
//! controllability over GCC: `-fno-align-loops` also suppresses the
//! 32-byte vector-loop alignment (GCC cannot turn that one off), so the
//! whole policy is A/B-testable and `-Os`-safe by construction.
//!
//! Alignment improves the front end on Raptor Lake because the DSB (uop
//! cache) tracks 32-byte windows inside 64-byte lines: a 32-byte aligned
//! vector-loop head always starts a fresh window, and a 16-byte aligned
//! scalar loop head keeps the loop inside a small number of fetch windows.
//! The measured lccc caveat (gzip `longest_match`, engineering ledger) was
//! about aligning *scalar* hot loops to 32 unconditionally — this policy
//! never does that.

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::ir::analysis::CfgAnalysis;
use crate::ir::instruction::{BasicBlock, Instruction, Terminator};
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
            AlignDirective { log2: 3, max_skip: None },
        ]
    }

    /// The vector-loop directive: 32 bytes, no skip clause — GCC 16.2 emits
    /// exactly this (as a 5/4/3 cascade that gas reduces to plain 32) for
    /// every vectorized loop header.
    fn vector_loop() -> AlignDirective {
        AlignDirective {
            log2: 5,
            max_skip: None,
        }
    }
}

/// Compute and record the alignment directives for every function in the
/// module. Pure analysis: the IR is not modified, and the recorded labels
/// are the final (post-renumber, post-PGO-layout) block labels.
pub fn align_module(module: &crate::ir::module::IrModule, cfg: &LoopAlignConfig) {
    let mut map: FxHashMap<u32, Vec<AlignDirective>> = FxHashMap::default();

    if std::env::var("CCC_NO_LOOP_ALIGN").is_ok() {
        record_directives(map);
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
        align_function(func, cfg, loops_on, joins_on, &pgo_map, &mut map);
    }

    record_directives(map);
}

fn align_function(
    func: &IrFunction,
    cfg: &LoopAlignConfig,
    loops_on: bool,
    joins_on: bool,
    pgo_map: &FxHashSet<u32>,
    map: &mut FxHashMap<u32, Vec<AlignDirective>>,
) {
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
        for lp in &loops {
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
            let directives = match cfg.loops {
                AlignControl::Custom { align, max_skip } => {
                    custom_directives(align, max_skip)
                }
                _ => {
                    let vector_body = lp.body.iter().any(|&bi| {
                        func.blocks
                            .get(bi)
                            .is_some_and(|b| block_has_vector_insn(b))
                    });
                    if vector_body {
                        vec![LoopAlignConfig::vector_loop()]
                    } else {
                        LoopAlignConfig::scalar_loop_cascade().to_vec()
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
                    AlignControl::Custom { align, max_skip } => {
                        custom_directives(align, max_skip)
                    }
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
    block.instructions.iter().any(|inst| {
        matches!(inst, Instruction::Intrinsic { op, .. } if op.is_vector_op())
    })
}
