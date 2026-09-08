//! Hot-loop alignment: bounded `.p2align` chains in front of loop headers.
//!
//! # Why loop alignment exists
//!
//! Every serious x86 compiler (GCC, Clang/LLVM, ICC, ICX) pads the first
//! instruction of a loop to an instruction-fetch boundary at `-O2`/`-O3`.
//! Measured on Compiler Explorer (see `engineering/` session notes for the
//! full transcripts):
//!
//! * GCC 16.2 `-O2/-O3` emits a `.p2align 5 / .p2align 4 / .p2align 3` chain
//!   before *innermost* loops (empirically: unconditional 32-byte alignment —
//!   GAS resolves the chain to its largest member), a 16/8 chain before
//!   *outer* loop headers, and the bounded form `.p2align 4,,10` +
//!   `.p2align 3` before scalar remainder loops. Constant tiny-trip loops
//!   (e.g. `for (i = 0; i < 3; ...)`) are left unaligned.
//! * Clang 23.1 `-O3` emits a plain `.p2align 4` before loop headers that
//!   survive MachineBlockPlacement's hotness gate.
//! * ICX emits `.p2align 4, 0x90`; ICC classic emits `.align 16,0x90` and
//!   even names its hot loop labels `..LNhot`.
//!
//! On Skylake-derived cores (incl. Raptor Lake) the dominant effects are:
//!
//! 1. **uop-cache (DSB) line granularity is 32 bytes.** A loop whose first
//!    instruction starts at a 32-byte boundary occupies a whole uop-cache
//!    line from its first byte; no line is wasted on pre-loop code.
//! 2. **The 16-byte decoder fetch windows.** A 16-aligned loop top never
//!    shares a fetch window with the preheader's tail, so the first
//!    iteration does not decode dead bytes.
//! 3. **The LSD** locks small aligned loops and replays them without
//!    re-fetching; alignment keeps the loop body contiguous in the uop cache.
//!
//! The cost is I-cache/DSB pressure and code size: every padding byte is a
//! byte of hot cache that executes only once (loop entry). That is why this
//! pass uses *bounded* chains (`.p2align N,,max`) instead of GCC 16's
//! empirically unbounded 32-byte padding: lccc's own PGO experiments
//! measured full 32-byte alignment of every hot header as SLOWER than
//! 16-byte alignment on gzip compress (multi-byte NOP bloat, ~11% growth of
//! `longest_match`), while GCC's and LLVM's defaults are 16-byte based.
//!
//! # Design
//!
//! * The analysis runs in the backend, at `generate_function` time, on the
//!   FINAL block order. Loop headers are found with the same
//!   dominator-backed natural-loop discovery the optimizer passes use
//!   (`passes::loop_analysis::find_merged_natural_loops`) — never by ad-hoc
//!   backward-edge guessing, which misclassifies irreducible edges.
//! * Policy is tiered by what the loop *is*:
//!   - **Innermost + vectorized body** (any `Vec*`/`Fma*`/vector-load
//!     intrinsic in the body): try 32 bytes with a 15-byte cap, falling back
//!     to 16 bytes with a 15-byte cap. Worst case equals plain 16-alignment;
//!     best case puts the hottest loops at a uop-cache line start. These are
//!     the loops the vectorizer produced precisely because they are hot.
//!   - **Other innermost loops**: plain 16-byte alignment (`.p2align 4`),
//!     matching Clang's measured default. Padding is inherently ≤ 15 bytes.
//!   - **Outer loop headers**: bounded `.p2align 4,,10` + `.p2align 3,,7`
//!     (GCC's bounded form). Outer headers execute once per *outer*
//!     iteration; the win is smaller, so the cap is tighter.
//!   * Loops with a visible constant trip count ≤ 4 stay unaligned (GCC and
//!     Clang both leave `for (i=0;i<3;i++)` alone; the padding cannot pay
//!     back over ≤4 iterations).
//! * `-falign-loops=N[:max]` overrides every tier with one user-specified
//!   chain (GCC-compatible subset); `-fno-align-loops` disables the pass.
//! * When PGO block-alignment data is active it takes precedence entirely:
//!   the profile already knows which loops are hot, and the 16-byte choice
//!   there is a measured result (see `pgo::layout`).
//! * `CCC_NO_ALIGN_LOOPS=1` disables the pass for A/B measurement;
//!   `CCC_ALIGN_DEBUG=1` prints one line per decision.
//!
//! The emitted directives are resolved by the assembler (GAS or lccc's own,
//! both support the three-operand form since this pass landed). Keeping the
//! decision as *directives* rather than pre-counted bytes means branch
//! relaxation inside the assembler can still shrink code without breaking
//! the alignment — the assembler re-derives padding after relaxation via
//! the align-marker fixup.

use crate::common::fx_hash::FxHashMap;
use crate::ir::analysis::CfgAnalysis;
use crate::ir::reexports::{IrFunction, Terminator};
use crate::passes::loop_analysis;

/// One bounded alignment tier: pad to `1 << log2` bytes unless that would
/// require more than `max_skip` bytes of padding (GAS `.p2align log2,,max`
/// semantics: the tier then does nothing at all).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AlignTier {
    pub log2: u8,
    pub max_skip: Option<u32>,
}

impl AlignTier {
    pub fn log2(log2: u8) -> Self {
        Self { log2, max_skip: None }
    }

    pub fn bounded(log2: u8, max_skip: u32) -> Self {
        Self {
            log2,
            max_skip: Some(max_skip),
        }
    }

    /// The directive text for this tier (without leading whitespace).
    pub fn directive(&self) -> String {
        match self.max_skip {
            Some(max) => format!(".p2align {},,{}", self.log2, max),
            None => format!(".p2align {}", self.log2),
        }
    }
}

/// The chain of tiers to emit immediately before one loop-header label.
/// Tiers are emitted in order; the assembler applies the first one whose
/// cap permits, later ones become no-ops on an already-aligned offset.
#[derive(Clone, Debug)]
pub(crate) struct LoopAlignPlan {
    pub tiers: Vec<AlignTier>,
}

impl LoopAlignPlan {
    pub fn new(tiers: Vec<AlignTier>) -> Self {
        Self { tiers }
    }

    pub fn is_empty(&self) -> bool {
        self.tiers.is_empty()
    }
}

/// Alignment policy: which tier chains apply to which loop class.
///
/// Built by the driver (see `CodegenOptions::align_loops`); the default
/// `-O2`/`-O3` x86 policy is [`LoopAlignPolicy::default_x86`].
#[derive(Clone, Debug)]
pub(crate) struct LoopAlignPolicy {
    /// Innermost loops whose body contains vector intrinsics.
    pub vector_chain: Vec<AlignTier>,
    /// Every other innermost loop.
    pub scalar_chain: Vec<AlignTier>,
    /// Headers of loops that contain other loops.
    pub outer_chain: Vec<AlignTier>,
}

impl LoopAlignPolicy {
    /// The measured default for x86-64/i686 at -O2/-O3 (see module docs).
    pub fn default_x86() -> Self {
        Self {
            vector_chain: vec![
                AlignTier::bounded(5, 15), // 32 B when reachable within 15 B
                AlignTier::bounded(4, 15), // else 16 B when reachable within 15 B
            ],
            scalar_chain: vec![AlignTier::log2(4)], // plain 16 B, ≤ 15 B padding
            outer_chain: vec![
                AlignTier::bounded(4, 10), // 16 B when ≤ 10 B
                AlignTier::bounded(3, 7),  // else 8 B when ≤ 7 B
            ],
        }
    }

    /// A single user-specified chain for every aligned loop
    /// (`-falign-loops=N[:max]`).
    pub fn uniform(log2: u8, max_skip: Option<u32>) -> Self {
        let tier = match max_skip {
            Some(max) => AlignTier::bounded(log2, max),
            None => AlignTier::log2(log2),
        };
        let chain = vec![tier];
        Self {
            vector_chain: chain.clone(),
            scalar_chain: chain.clone(),
            outer_chain: chain,
        }
    }
}

/// Per-function analysis result: block label id → alignment plan.
pub(crate) struct LoopAlignAnalysis {
    pub plans: FxHashMap<u32, LoopAlignPlan>,
}

impl LoopAlignAnalysis {
    pub fn plan_for(&self, label: u32) -> Option<&LoopAlignPlan> {
        self.plans.get(&label)
    }
}

/// Whether an intrinsic op is a vector/SIMD operation, i.e. evidence that
/// the loop body was produced by (or for) the vectorizer and runs in
/// xmm/ymm registers.
///
/// The `Vec*` family has 100+ variants and grows with every vectorizer
/// feature; matching on the Debug name keeps this predicate total over
/// future variants instead of silently missing new ones (a missed variant
/// would only downgrade a vector loop to the scalar chain, but a stale
/// hard-coded list is still a bug magnet).
fn intrinsic_is_vector(op: &crate::ir::intrinsics::IntrinsicOp) -> bool {
    let name = format!("{op:?}");
    name.starts_with("Vec")
        || name.starts_with("FmaF")
        || name.starts_with("BroadcastLoad")
        || matches!(
            op,
            crate::ir::intrinsics::IntrinsicOp::LoadF64x4
                | crate::ir::intrinsics::IntrinsicOp::LoadF64x2
                | crate::ir::intrinsics::IntrinsicOp::LoadI32x8
                | crate::ir::intrinsics::IntrinsicOp::LoadI32x4
                | crate::ir::intrinsics::IntrinsicOp::Loaddqu
                | crate::ir::intrinsics::IntrinsicOp::Storedqu
        )
}

/// Visible constant trip count of a counted loop from its *exit* comparison:
/// `Some(n)` when the compare that feeds the header's conditional branch is
/// `< Const(n)` (or the mirrored `Const(n) >` form). Comparisons that do not
/// guard the backedge are ignored — a stray constant compare inside the
/// header must not disqualify alignment.
/// Remainder loops divided by the vector width keep their scaled bound, so
/// this deliberately measures the *runtime* iteration count, which is what
/// alignment payback depends on.
fn header_constant_trip_bound(func: &IrFunction, header_idx: usize) -> Option<i64> {
    let header = func.blocks.get(header_idx)?;
    let exit_cond = match &header.terminator {
        Terminator::CondBranch { cond, .. } => match cond {
            crate::ir::reexports::Operand::Value(v) => Some(*v),
            _ => None,
        },
        _ => None,
    }?;
    for inst in &header.instructions {
        if let crate::ir::reexports::Instruction::Cmp {
            dest,
            op,
            lhs,
            rhs,
            ..
        } = inst
        {
            if *dest != exit_cond {
                continue;
            }
            let (limit, flipped) = match rhs {
                crate::ir::reexports::Operand::Const(c) => (c, false),
                // The mirrored form `Const(n) > iv` bounds the trip count too.
                crate::ir::reexports::Operand::Value(_) => match lhs {
                    crate::ir::reexports::Operand::Const(c) => (c, true),
                    _ => continue,
                },
            };
            let limit = limit.to_i64()?;
            let is_lt = match op {
                crate::ir::ops::IrCmpOp::Slt | crate::ir::ops::IrCmpOp::Ult => !flipped,
                crate::ir::ops::IrCmpOp::Sgt | crate::ir::ops::IrCmpOp::Ugt => flipped,
                _ => continue,
            };
            if is_lt {
                return Some(limit);
            }
        }
    }
    None
}

/// Analyze one function and produce the alignment plans for its loop
/// headers. Returns `None` when nothing should be aligned (no policy, no
/// loops, cold section, or the A/B kill switch is set).
pub(crate) fn analyze_function(
    func: &IrFunction,
    policy: &LoopAlignPolicy,
) -> Option<LoopAlignAnalysis> {
    if std::env::var("CCC_NO_ALIGN_LOOPS").is_ok() {
        return None;
    }
    // Cold functions were placed in .text.unlikely by PGO layout; padding
    // code that rarely executes is pure I-cache waste.
    if func
        .section
        .as_deref()
        .is_some_and(|s| s.contains("unlikely"))
    {
        return None;
    }
    if func.blocks.len() < 2 {
        return None;
    }

    let debug = std::env::var("CCC_ALIGN_DEBUG").is_ok();
    let cfg = CfgAnalysis::build(func);
    let loops = loop_analysis::find_merged_natural_loops(
        cfg.num_blocks,
        &cfg.preds,
        &cfg.succs,
        &cfg.idom,
    );
    if loops.is_empty() {
        return None;
    }
    let nest = loop_analysis::LoopNest::analyze(cfg.num_blocks, &loops);

    let mut plans: FxHashMap<u32, LoopAlignPlan> = FxHashMap::default();
    for (loop_idx, natural_loop) in loops.iter().enumerate() {
        let header_idx = natural_loop.header;
        let header_label = func.blocks[header_idx].label;
        let is_innermost = nest.children[loop_idx].is_empty();
        // Which chain applies: vectorized inner loops are the hottest code
        // the compiler generates and get the 32-byte attempt; other loops
        // get the conservative chains.
        let has_vector_body = natural_loop.body.iter().any(|&block_idx| {
            func.blocks[block_idx].instructions.iter().any(|inst| {
                matches!(
                    inst,
                    crate::ir::reexports::Instruction::Intrinsic { op, .. }
                        if intrinsic_is_vector(op)
                )
            })
        });
        // Loops that visibly run ≤ 4 iterations never pay the padding back
        // (GCC/Clang leave constant-trip-3 loops unaligned too).
        if let Some(bound) = header_constant_trip_bound(func, header_idx) {
            if bound <= 4 {
                if debug {
                    eprintln!(
                        "[align] {} loop header block {}: tiny trip bound {} — skipped",
                        func.name, header_idx, bound
                    );
                }
                continue;
            }
        }
        let chain: &[AlignTier] = if has_vector_body && is_innermost {
            &policy.vector_chain
        } else if is_innermost {
            &policy.scalar_chain
        } else {
            &policy.outer_chain
        };
        if chain.is_empty() {
            continue;
        }
        if debug {
            let class = if has_vector_body && is_innermost {
                "inner-vector"
            } else if is_innermost {
                "inner-scalar"
            } else {
                "outer"
            };
            eprintln!(
                "[align] {} loop header block {} ({}): {}",
                func.name,
                header_idx,
                class,
                chain
                    .iter()
                    .map(|t| t.directive())
                    .collect::<Vec<_>>()
                    .join(" + ")
            );
        }
        plans
            .entry(header_label.0)
            .or_insert_with(|| LoopAlignPlan::new(chain.to_vec()));
    }

    if plans.is_empty() {
        None
    } else {
        Some(LoopAlignAnalysis { plans })
    }
}
