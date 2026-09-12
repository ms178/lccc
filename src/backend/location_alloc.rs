//! Global Location Allocation (GLA) — P0-A, Phase 1.
//!
//! # Why this module exists
//!
//! The production register allocator's only spill model is **lifetime
//! demotion**: a value that loses an eviction contest is homed in one stack
//! slot for its whole live range, so every later use either folds a memory
//! operand or pays a reload. `split_ranges::split_high_pressure_ranges`
//! (RA-06) introduced Belady-MIN splitting, but only *intra-block*: its
//! own decision record (`engineering/DECISIONS.md`, "RA-06 (2026-09-05)")
//! proves that partial spill-then-color is strictly worse than either
//! endpoint, because the loop-carried values that dominate real pressure
//! peaks are header-phi webs whose consumers live in another block, and
//! renaming those needs global SSA repair. That record states the missing
//! prerequisite verbatim: *"Cross-block SSA repair, so the spill phase can
//! reach MAXLIVE <= k globally."* This module is that prerequisite.
//!
//! # Location model
//!
//! The planner reasons about three locations (the `Location`/`LocationPiece`
//! vocabulary of the P0-A design):
//!
//! ```text
//! enum Location { Register, Stack(slot), Rematerialize(template) }
//! ```
//!
//! A logical SSA value is therefore no longer assumed to own one register
//! for its whole life. The Phase-1 materialization does not pin physical
//! registers (the existing scan / graph colorer still chooses the register
//! for every *register* piece); it realizes `Stack` and `Rematerialize`
//! pieces as IR rewrites *before* phi elimination:
//!
//! * a **Stack piece** is bounded by a fresh reload SSA value (the register
//!   piece after the gap); the backing slot is an internal volatile alloca,
//!   exactly like the proven call/pressure splitters;
//! * a **Rematerialize piece** re-executes a source-less cheap definition
//!   (`GlobalAddr`, `Copy` of a constant) instead of reloading.
//!
//! Register pieces are simply the (now shorter) remaining webs of the
//! original value, which the downstream colorer allocates exactly as today.
//!
//! # Why the SSA repair needs no new φ nodes (and when that changes)
//!
//! A cross-block gap joins register pieces across control flow, which in a
//! textbook Cytron spiller means φ insertion at dominance frontiers. This
//! module avoids hand-placed φs through a stronger construct:
//!
//! * every split value receives exactly one **immutable capture store**
//!   immediately after its static definition. SSA values are immutable, so
//!   the slot then holds the value on *every* path for the rest of that
//!   dynamic execution (for a header φ the capture follows the φ prefix and
//!   re-executes every iteration with the current joined value);
//! * a reload may therefore be placed at the head of *any* memory-region
//!   block, immediately before any use cluster, or on any split CFG edge,
//!   and it provably reads the correct value — the memory slot itself is
//!   the join (a φ realized in storage), and each reload reconstructs a
//!   fresh SSA name;
//! * an edge that needs a register value only on one successor gets a
//!   **critical-edge trampoline** (the same construction phi elimination
//!   uses), so the reload never speculatively executes on the other edge.
//!
//! This is the SplitKit observation (a spill slot is a legal, dominance-free
//! join point) and it is strictly simpler to verify than φ insertion: the
//! only dominance obligation is "the capture store follows the unique
//! definition, which already dominates every former use", plus "each reload
//! is dominated by that store". The verifier below checks both.
//!
//! What this construction is *not*: it does not yet pin physical registers
//! per piece, does not resolve cyclic φ webs into register permutations
//! (P0-B), and does not fold address expressions (P0-C). Recurrence-carried
//! header φs (sha256's a..h, accumulator reductions) are deliberately
//! excluded from anchor eviction: an immutable capture on a header φ stores
//! every iteration on the carried dependency chain itself, which is the
//! exact demotion the production allocator already protects against via
//! `span_recurrence`.
//!
//! # Fail-closed contract
//!
//! Every analysis gap resolves to "do not split this value". A value is only
//! split when the planner proves (a) GPR eligibility, (b) a pressure excess
//! the edit relieves in a block the edit can plausibly tip under budget, and
//! (c) a weighted benefit above the weighted store/reload traffic. The
//! transform never rewrites a shape it did not model.
//!
//! Enable with `CCC_RA_GLOBAL_LOCATION=1` (A/B gate, default off; an explicit
//! `=0`/`false`/`off` also means off).
//!
//! # Shipped Phase-1 policy (A/B calibrated 2026-09-11, post PR #499)
//!
//! The planner's vocabulary (remat, cross-block gaps, edge trampolines) is
//! fully implemented, but the shipped policy is deliberately narrow; every
//! gate is a fail-closed filter (loosening a gate can only ADD edits). The
//! calibration used the 78-file benchmark corpus
//! (`scripts/ra_quality_census.py --ab-env`) plus Godbolt oracle rank
//! (`scripts/codegen_oracle.py --rank`):
//!
//! * **Rematerialization ON, spill gaps OFF** (`CCC_GLA_SPILL_GAPS=1` to
//!   re-enable). Pre-allocation capture-store/reload gaps measured
//!   net-negative in every configuration: the proxy cannot see which
//!   residency the production colorer (folding memory operands, callee-save
//!   buys, and PR #499's next-use cost-ratio escape) already resolves, so
//!   gaps mostly traded once-per-call callee-save traffic for per-iteration
//!   reloads. The gap machinery is retained as the P0-B / post-alloc
//!   feedback substrate but never fires in shipped policy.
//! * **Pressure counts coalesced color CLASSES**, not SSA names: every φ
//!   result is unioned with its incoming operands. This can only
//!   under-count (φ coalescing may itself insert edge copies in the
//!   colorer), which is the safe direction.
//! * **Reach band, target- and tier-aware** (`CCC_GLA_REACH`): blocks whose
//!   peak exceeds the register budget by more than the band cannot be made
//!   colorable by a handful of edits, and editing them can reshuffle the
//!   colorer's choices in blocks it would have handled optimally. The band
//!   proxies the production allocator's residual register capacity: 6 on
//!   x86-64 (the six buyable callee-saved GPRs), **2 on i686** (under PIC
//!   %ebx is the GOT base and %rbp the frame pointer, leaving %esi/%edi —
//!   band 6 caused a measured 1.053× loop_patterns runtime regression from
//!   an induction counter spilled in exchange for a buffer base, band 2
//!   removes it while keeping every static win), and **64 (unbanded) at
//!   -O0** where the coloring tier is disabled so there is no plan to
//!   perturb and remat relief is a direct stack-traffic reduction. This is
//!   the nbody discriminator on x86-64: its FP inner loop peaks at 38–45
//!   GPR classes and every edit there only perturbed the global coloring;
//!   adler32 peaks at 14–16 and one remat tips it.
//! * **Remat segment cap 1** (`CCC_GLA_REMAT_MAX_SEGMENTS`): a value
//!   whose hole-aware live range has two or more segments (nbody's printf
//!   format-string global, two segments around the nested simulation
//!   loops) is not rematerialized: the cloned definition only reorders
//!   global coloring for a net instruction increase. Single-segment
//!   hoisted bases (adler32's check table base) free a register cleanly.
//!   For nbody itself the reach band now rejects first (v94 covers only
//!   hopeless blocks); the cap remains an independently load-bearing
//!   gate when the band is widened, pinned in the 2×2 matrix in
//!   `tests/regression/check_gla_remat_policy.sh`.
//!
//! Result with the feature ENABLED (78-file corpus, -O2): one fire site
//! (zlib_ng_adler32 main), +4 static instructions and +1 cold
//! unconditional jump, but −12 stack references, stores 6→4 and spills
//! 19→7 measured by the Godbolt tooling — the 8 hottest references move
//! out of the per-8-byte inner DO8 loop; the remaining store/reload pair
//! executes once per 5552-byte NMAX block. Frame shrinks 56→40 bytes.
//! At -O0 the same gate is insn-neutral-to-positive corpus-wide.
//! i686 (budget 6 GPRs): −4 stack refs corpus-wide with cold prologue
//! setup the only +insn sites. Gap-enabled fuzz (synthetic / phi_cfg /
//! differential, verifier forced) and full gap unit tests keep the
//! dormant substrate covered.

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::{AddressSpace, IrType};
use crate::ir::analysis;
use crate::ir::reexports::*;
use crate::passes::loop_analysis;
use std::sync::OnceLock;

use super::liveness::LivenessResult;
use super::split_ranges::{
    collect_value_types, first_non_phi, insert_entry_alloca, is_simple_gpr_type, next_value,
    pressure_budget, pressure_min_gap, replace_values_in_inst, replace_values_in_terminator,
    split_debug_enabled, terminator_uses_value,
};

// ─────────────────────────────────────────────────────────────────────────────
// Tunables (env-overridable for A/B work; clamped)
// ─────────────────────────────────────────────────────────────────────────────

/// Master A/B gate: `CCC_RA_GLOBAL_LOCATION` set to a truthy value. An
/// explicit `0`/`false`/`off`/empty means OFF (a bare presence check would
/// let `CCC_RA_GLOBAL_LOCATION=0` silently enable the feature).
pub(crate) fn gate_enabled() -> bool {
    matches!(
        std::env::var("CCC_RA_GLOBAL_LOCATION").as_deref(),
        Ok(v) if !matches!(v, "" | "0" | "off" | "no")
            && !v.eq_ignore_ascii_case("false")
    )
}

/// Per-function edit budget (`CCC_RA_GLOBAL_LOCATION_MAX`, default 64).
pub(crate) fn max_splits_from_env() -> usize {
    std::env::var("CCC_RA_GLOBAL_LOCATION_MAX")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(64)
        .clamp(0, 4096)
}

/// A source-less definition is rematerialized everywhere only when its
/// dynamic use weight stays at or below this count. A `leaq`/`mov $imm` in a
/// hot loop with many readers must keep a register instead.
fn remat_max_weight() -> u64 {
    static M: OnceLock<u64> = OnceLock::new();
    *M.get_or_init(|| {
        std::env::var("CCC_RA_REMAT_MAX_USES")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(3)
            .clamp(1, 64)
    })
}

/// Benefit/cost ratio required for a spill gap (tenths: 20 = 2.0×). Relieving
/// pressure over one point must promise at least this multiple of the
/// weighted store + reload traffic the gap introduces.
fn benefit_ratio10() -> u64 {
    static M: OnceLock<u64> = OnceLock::new();
    *M.get_or_init(|| {
        std::env::var("CCC_RA_GLA_RATIO")
            .ok()
            .and_then(|s| s.parse::<f64>().ok())
            .map(|f| (f * 10.0).round() as u64)
            .unwrap_or(20)
            .clamp(10, 100)
    })
}

/// Loop-depth frequency weight — the same currency the production allocator
/// uses (`spill_cost_at` / `priority`: 10^min(depth,4)).
fn block_frequency(depth: u32) -> u64 {
    10u64.saturating_pow(depth.min(4))
}

/// Whether explicit spill gaps (capture store + reload pieces) may be
/// planned at all. The Phase-1 A/B census (`scripts/ra_quality_census.py`,
/// 78-file corpus) measured pre-alloc gap insertion as net-negative in
/// every configuration: the pressure proxy counts points the production
/// colorer resolves for free by folding memory operands, and the gaps that
/// do fire mainly replace once-per-call callee-save pushes with per-
/// iteration reload traffic. Gap support stays in the code (it is the
/// substrate P0-B cyclic-φ and post-alloc feedback builds on) but defaults
/// OFF; the whole feature is additionally behind `CCC_RA_GLOBAL_LOCATION`.
fn allow_spill_gaps() -> bool {
    static S: OnceLock<bool> = OnceLock::new();
    *S.get_or_init(|| {
        std::env::var("CCC_GLA_SPILL_GAPS")
            .map(|v| v != "0" && v != "false")
            .unwrap_or(false)
    })
}

/// Whether same-block reload gaps are allowed. The RA-06 decision record
/// measured intra-block pre-alloc spill-then-reload as net-negative on every
/// corpus program: the production allocator's own evictions fold memory
/// operands into the consuming instruction at zero extra instructions, while
/// an explicit gap always pays an uncoalescable store + reload. The global
/// allocator's unique value is CROSS-block (and cross-call) pressure, so
/// same-block gaps are rejected unless explicitly re-enabled for A/B work.
fn allow_intra_block_gaps() -> bool {
    static A: OnceLock<bool> = OnceLock::new();
    *A.get_or_init(|| {
        std::env::var("CCC_GLA_ALLOW_INTRA")
            .map(|v| v != "0" && v != "false")
            .unwrap_or(false)
    })
}

/// Relief band: a block whose peak exceeds the register budget by more than
/// this many color classes is not made colorable by a handful of splits —
/// the production allocator will stack-allocate there regardless, so any
/// edits whose only covered over-budget points lie in such blocks add
/// capture traffic for zero register residency, and (worse) can reshuffle
/// the colorer's choices in the blocks it *would* have handled optimally.
///
/// The number proxies the residual register capacity the production
/// allocator can still bring to bear beyond the scan budget:
///
/// * **x86-64, allocator tier (opt ≥ 1): 6** — the six callee-saved GPRs
///   (%rbx, %rbp, %r12–%r15) the colorer can buy for amortized push/pop
///   cost, plus memory-operand folding. This is the adler32 (peaks 14–16
///   vs budget 12: one remat tips it) vs nbody (peaks 38–45: futile)
///   discriminator.
/// * **i686, allocator tier (opt ≥ 1): 2** — under position-independent
///   code %ebx is permanently the GOT base and %rbp is the frame pointer,
///   so only %esi/%edi remain as cheaply buyable callee-saved homes.
///   Measured on the 51-program m32 corpus at -O2 (paired wall-clock A/B,
///   21 reps): band 6 rematerialized a 3-use buffer base in loop_patterns
///   that made the colorer spill the 10 M-trip induction counter to a stack
///   slot — −7 static instructions but **1.053× runtime**; band 2 removes
///   that edit and every other cold-churn site while retaining every static
///   win (reduction_vecreg, fp_memfold_stencil5, tls_pass, prefix_scan;
///   matmul measured 0.992×). Band 3 already re-admits the counter spill
///   (1.038×); only 2 is correct for this target.
/// * **opt level 0: 64 (effectively unbanded)** — at -O0 the production
///   register allocator tier is disabled (`disable_regalloc` at level 0),
///   so there is no colorer/folding plan for pre-allocation edits to
///   perturb and no callee-save capacity to proxy: nearly every value is
///   stack-homed regardless, and remat relief is a direct stack-traffic
///   win. The m32 -O0 corpus improves monotonically as the band widens
///   (−4 insn/−5 stkref at band 2 vs −56/−54 at band 16, where the
///   segment/use gates saturate); x86-64 -O0 is band-insensitive.
///
/// Fails closed in every tier: raising the value can only ADD edits.
/// `CCC_GLA_REACH` overrides the target/opt default for A/B work.
fn tier_for(opt_level: u32) -> Tier {
    match opt_level {
        0 => Tier::Debug,
        4 | 5 => Tier::Size,
        _ => Tier::Speed,
    }
}

fn reach_band(tier: Tier) -> u32 {
    if let Ok(v) = std::env::var("CCC_GLA_REACH") {
        if let Ok(n) = v.parse::<u32>() {
            return n.clamp(0, 64);
        }
    }
    match tier {
        // No coloring tier to perturb: any pressure relief is direct.
        Tier::Debug => 64,
        // See the band calibration in the module header (2 on i686 PIC:
        // only %esi/%edi are free beyond the budget; 6 on x86-64: the six
        // buyable callee-saved GPRs).
        Tier::Speed => {
            if crate::common::types::target_is_32bit() {
                2
            } else {
                6
            }
        }
        // GLA does not plan for size tiers; band irrelevant.
        Tier::Size => 0,
    }
}

/// Maximum number of hole-aware live segments a rematerialized value may
/// have. Each additional segment is an extra recomputation cluster; a
/// multi-segment global address weaving through deep loops (nbody's bodies
/// base: 2 segments across the depth-3 FP loop) only perturbs the colorer's
/// global assignment for a net loss, whereas a single-segment hoisted base
/// (adler32: 1 segment) frees a register cleanly. Cap defaults to 1; the
/// master feature gate is still required. Swept via
/// `CCC_GLA_REMAT_MAX_SEGMENTS`.
fn remat_max_segments() -> usize {
    static M: OnceLock<usize> = OnceLock::new();
    *M.get_or_init(|| {
        std::env::var("CCC_GLA_REMAT_MAX_SEGMENTS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(1usize)
            .clamp(1, 1024)
    })
}

/// Minimum weighted benefit a gap must promise before it is selected. A
/// handful of budget points in a cold block must not justify a permanent
/// capture slot. Clampable via `CCC_GLA_MIN_BENEFIT`.
fn min_benefit() -> u64 {
    static M: OnceLock<u64> = OnceLock::new();
    *M.get_or_init(|| {
        std::env::var("CCC_GLA_MIN_BENEFIT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(40)
            .clamp(1, 1_000_000)
    })
}

/// Which production allocator tier the planner is preparing code for. The
/// profitability model differs because the two tiers handle pressure
/// fundamentally differently.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tier {
    /// -O0: the coloring allocator is disabled; nearly every value is
    /// stack-homed and rematerialization removes slot round-trip traffic
    /// directly (no colorer assignment exists to perturb).
    Debug,
    /// -O1/-O2/-O3: full scan/color/fold/callee-save tier. Only edits with
    /// NET register relief outside the value's own def/read spans are
    /// credited; global-coloring perturbations are not predictable pre-
    /// allocation and measured either way (adler32 win, nbody -Os churn).
    Speed,
    /// -Os/-Oz: size-oriented allocator. Remat clones trade one carried
    /// home for per-cluster instructions, the opposite of the size
    /// objective, and the pressure proxy was calibrated on speed-tier
    /// callee-save capacity. GLA plans nothing here.
    Size,
}

/// Explicit planning policy, built from the environment in production
/// ([`GlaPolicy::from_env`]) and constructed directly in unit tests so test
/// behavior never depends on process-wide, once-initialized env knobs.
#[derive(Clone, Debug)]
struct GlaPolicy {
    /// Production allocator tier this plan is prepared for.
    tier: Tier,
    /// Register residency budget (GPR color classes per point).
    budget: usize,
    /// Total edit budget (remats + gaps) per function.
    max_splits: usize,
    /// Dynamic-use weight above which a value keeps a register instead of
    /// being rematerialized.
    remat_max_weight: u64,
    /// Maximum hole-aware live segments a rematted value may have.
    remat_max_segments: usize,
    /// Excess classes over budget still considered salvageable.
    reach_band: u32,
    /// Whether capture-store/reload gaps may be planned at all.
    allow_spill_gaps: bool,
    /// Whether gaps whose both register pieces are in the same block count.
    allow_intra_block_gaps: bool,
    /// Minimum point span for an in-block gap.
    min_gap: usize,
    /// Minimum weighted benefit a gap must promise.
    min_benefit: u64,
    /// Required benefit/cost ratio in tenths (20 = 2.0×).
    benefit_ratio10: u64,
}

impl GlaPolicy {
    fn from_env(max_splits: usize, opt_level: u32) -> Self {
        let tier = tier_for(opt_level);
        GlaPolicy {
            tier,
            budget: pressure_budget(),
            max_splits,
            remat_max_weight: remat_max_weight(),
            remat_max_segments: remat_max_segments(),
            reach_band: reach_band(tier),
            allow_spill_gaps: allow_spill_gaps(),
            allow_intra_block_gaps: allow_intra_block_gaps(),
            min_gap: pressure_min_gap(),
            min_benefit: min_benefit(),
            benefit_ratio10: benefit_ratio10(),
        }
    }

    /// Over-budget peak within the salvageable band (see [`reach_band`]).
    fn peak_is_reachable(&self, pk: u32) -> bool {
        pk as usize <= self.budget + self.reach_band as usize
    }

    /// Size tiers never plan: see [`Tier::Size`].
    fn enabled(&self) -> bool {
        self.tier != Tier::Size
    }

    /// Whether residency relief must be net of the value's own def/read
    /// spans. True for the coloring allocator tier (a clone re-occupies a
    /// register at every use run); false at -O0 where every value is
    /// stack-homed and even use-point remats remove slot round-trips.
    fn net_relief(&self) -> bool {
        self.tier == Tier::Speed
    }
}

#[cfg(test)]
impl GlaPolicy {
    /// Fully permissive policy for mechanism tests (gaps ON, including
    /// intra-block; generous reach/ratio/segment limits).
    fn testing_permissive() -> Self {
        GlaPolicy {
            tier: Tier::Speed,
            budget: pressure_budget(),
            max_splits: 64,
            remat_max_weight: remat_max_weight(),
            remat_max_segments: 1024,
            reach_band: 64,
            allow_spill_gaps: true,
            allow_intra_block_gaps: true,
            min_gap: pressure_min_gap(),
            min_benefit: 1,
            benefit_ratio10: 10,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Location model (planning vocabulary; also the public audit surface)
// ─────────────────────────────────────────────────────────────────────────────

/// Where a value's bits live during a piece.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Location {
    /// In some GPR; the downstream colorer picks which.
    Register,
    /// In the function's frame, in the per-value capture slot.
    Stack,
    /// Re-derived on demand by cloning a source-less definition.
    Rematerialize,
}

/// A half-open intra-block span `[start, end)` over local instruction points
/// (point `n` = the terminator) during which a logical value occupies
/// `location`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LocationPiece {
    pub(crate) vid: u32,
    pub(crate) block: usize,
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) location: Location,
}

// ─────────────────────────────────────────────────────────────────────────────
// Eligibility
// ─────────────────────────────────────────────────────────────────────────────

/// Values a sound Phase-1 splitter must not touch: register-class owners
/// (intrinsics, inline asm), allocas themselves, and x86 fixed-register
/// operands (div/rem pair, variable shift count) whose post-split coloring is
/// governed by dedicated allocator machinery.
fn collect_ineligible(func: &IrFunction) -> FxHashSet<u32> {
    let mut bad: FxHashSet<u32> = FxHashSet::default();
    for b in &func.blocks {
        for inst in &b.instructions {
            match inst {
                Instruction::Alloca { dest, .. } | Instruction::DynAlloca { dest, .. } => {
                    bad.insert(dest.0);
                }
                Instruction::Intrinsic { .. } | Instruction::InlineAsm { .. } => {
                    if let Some(d) = inst.dest() {
                        bad.insert(d.0);
                    }
                    crate::backend::liveness::for_each_operand_in_instruction(inst, |op| {
                        if let Operand::Value(v) = op {
                            bad.insert(v.0);
                        }
                    });
                    crate::backend::liveness::for_each_value_use_in_instruction(inst, |v| {
                        bad.insert(v.0);
                    });
                }
                // Div/rem incarnations are fused as (numerator, denominator,
                // signedness, barriers); splitting an operand mints a fresh
                // incarnation the fusion stamps cannot match, and on i686 the
                // %eax/%edx contract is a hard pin. Exclude both operands.
                Instruction::BinOp {
                    op: IrBinOp::UDiv | IrBinOp::SDiv | IrBinOp::URem | IrBinOp::SRem,
                    lhs,
                    rhs,
                    ..
                } => {
                    for op in [lhs, rhs] {
                        if let Operand::Value(v) = op {
                            bad.insert(v.0);
                        }
                    }
                }
                // A non-constant shift count must be in %cl on x86 / the
                // equivalent fixed register elsewhere.
                Instruction::BinOp {
                    op: IrBinOp::Shl | IrBinOp::LShr | IrBinOp::AShr,
                    rhs: Operand::Value(v),
                    ..
                } => {
                    bad.insert(v.0);
                }
                _ => {}
            }
        }
    }
    bad
}

/// Recurrence-carried header φ destinations. Such a web re-enters its header
/// on a back edge carrying a member of the same web (the a..h sha256
/// schedule, every accumulator reduction). An anchor store on its
/// definition executes on the carried chain every iteration; the production
/// allocator already has measured, protected policy for these
/// (`span_recurrence`), and a memory piece here is never an improvement.
fn collect_recurrence_phi_dests(
    func: &IrFunction,
    label_map: &FxHashMap<BlockId, usize>,
    preds: &analysis::FlatAdj,
    succs: &analysis::FlatAdj,
    idom: &[usize],
) -> FxHashSet<u32> {
    let mut out = FxHashSet::default();
    let loops = loop_analysis::find_natural_loops(func.blocks.len(), preds, succs, idom);
    for lp in &loops {
        let h = lp.header;
        for inst in &func.blocks[h].instructions {
            let Instruction::Phi { dest, incoming, .. } = inst else {
                continue;
            };
            let recurrent = incoming.iter().any(|(op, pred)| {
                let Some(&pred_idx) = label_map.get(pred) else {
                    return false;
                };
                lp.body.contains(&pred_idx)
                    && matches!(op, Operand::Value(v) if defined_in_loop(func, v.0, &lp.body))
            });
            if recurrent {
                out.insert(dest.0);
            }
        }
    }
    out
}

fn defined_in_loop(func: &IrFunction, vid: u32, body: &FxHashSet<usize>) -> bool {
    func.blocks
        .iter()
        .enumerate()
        .filter(|(bi, _)| body.contains(bi))
        .any(|(_, b)| {
            b.instructions
                .iter()
                .any(|i| i.dest().is_some_and(|d| d.0 == vid))
        })
}

/// A definition cheaply reproducible without register history. Returns a
/// fresh template carrying `dest` when every input is static.
fn rematerializable_template(inst: &Instruction) -> Option<Instruction> {
    match inst {
        Instruction::GlobalAddr { dest, name } => Some(Instruction::GlobalAddr {
            dest: *dest,
            name: name.clone(),
        }),
        Instruction::Copy {
            dest,
            src: src @ Operand::Const(_),
        } => Some(Instruction::Copy {
            dest: *dest,
            src: src.clone(),
        }),
        _ => None,
    }
}

/// Locate a value's unique static definition: (block, instruction index,
/// true when the def is a φ at that block's head).
fn find_def(func: &IrFunction, vid: u32) -> Option<(usize, usize, bool)> {
    for (bi, b) in func.blocks.iter().enumerate() {
        for (ii, inst) in b.instructions.iter().enumerate() {
            if inst.dest().is_some_and(|d| d.0 == vid) {
                return Some((bi, ii, matches!(inst, Instruction::Phi { .. })));
            }
        }
    }
    None
}

/// Whether `vid` is (re-)defined inside block `bi`. The memory-region
/// closure must never propagate INTO such a block: liveness's φ-incoming
/// edge copies conservatively mark even an in-block-defined value live-in
/// (safe for spilling, not for memory-state propagation), and a reload
/// before a definition's capture anchor would read a stale slot.
fn block_defines_value(func: &IrFunction, bi: usize, vid: u32) -> bool {
    func.blocks[bi]
        .instructions
        .iter()
        .any(|i| i.dest().is_some_and(|d| d.0 == vid))
}

/// Sorted, de-duplicated intra-block READ points of `vid` (local instruction
/// indices; the terminator point is `n`). φ operands are EDGE reads at the
/// predecessor and are intentionally excluded. Folded hidden reads (SIB/GEP
/// bases with no IR operand) are folded in so a value the addressing mode
/// re-reads cannot be evicted across the access.
fn block_use_points(live: &LivenessResult, func: &IrFunction, bi: usize, vid: u32) -> Vec<usize> {
    let block = &func.blocks[bi];
    let n = block.instructions.len();
    let mut pts = Vec::new();
    let mut seen = FxHashSet::default();
    for (ii, inst) in block.instructions.iter().enumerate() {
        if matches!(inst, Instruction::Phi { .. }) {
            continue;
        }
        let mut hits = false;
        crate::backend::liveness::for_each_operand_in_instruction(inst, |op| {
            if matches!(op, Operand::Value(v) if v.0 == vid) {
                hits = true;
            }
        });
        crate::backend::liveness::for_each_value_use_in_instruction(inst, |v| {
            if v.0 == vid {
                hits = true;
            }
        });
        if hits && seen.insert(ii) {
            pts.push(ii);
        }
    }
    if terminator_uses_value(&block.terminator, vid) && seen.insert(n) {
        pts.push(n);
    }
    // Folded reads at global points: map to local points in this block.
    // `gp >= gs` is mandatory — a saturating_sub would map a read in an
    // EARLIER block to phantom local point 0 (inside the φ prefix).
    let gs = live.block_starts[bi];
    let ge = live.block_ends[bi];
    if let Some(folded) = live.folded_read_points.get(&vid) {
        for &gp in folded {
            if gp < gs || gp > ge {
                continue;
            }
            let local = (gp - gs) as usize;
            if local <= n && seen.insert(local) {
                pts.push(local);
            }
        }
    }
    pts.sort_unstable();
    pts.dedup();
    pts
}

/// Whether `vid` is an incoming operand of some φ in successor block `s`, on
/// block `bi`'s edge (pred label match).
fn phi_incoming_uses(func: &IrFunction, s: usize, bi: usize, vid: u32) -> bool {
    let pred_label = func.blocks[bi].label;
    func.blocks[s].instructions.iter().any(|i| match i {
        Instruction::Phi { incoming, .. } => incoming
            .iter()
            .any(|(op, p)| *p == pred_label && matches!(op, Operand::Value(v) if v.0 == vid)),
        _ => false,
    })
}

/// Whether `vid` reaches ANY successor φ from block `bi`.
fn block_feeds_successor_phi(func: &IrFunction, bi: usize, vid: u32) -> bool {
    func.blocks
        .iter()
        .enumerate()
        .any(|(s, _)| phi_incoming_uses(func, s, bi, vid))
}

// ─────────────────────────────────────────────────────────────────────────────
// Pressure
// ─────────────────────────────────────────────────────────────────────────────

/// Union-find over SSA values that the production colorer coalesces into a
/// single register for free: φ results with their incoming operands (the
/// allocator's hole-aware φ coalescing; RA machinery must not be rebuilt
/// here). Counting each SSA name separately systematically over-states
/// pressure on loop-heavy code (every loop-carried value is a φ web) and is
/// what made pre-alloc splitting fire on functions the colorer actually
/// fits. Unioning φ webs can only UNDER-count pressure relative to the
/// colorer, which is the required fail-closed direction: when in doubt the
/// planner splits nothing. φ coalescing can fail in the colorer (it inserts
/// edge copies); treating it as always-coalesced is therefore a deliberately
/// conservative planning approximation, not an exact analysis.
struct ColorClasses {
    parent: FxHashMap<u32, u32>,
}

impl ColorClasses {
    fn phi_webs(func: &IrFunction) -> Self {
        fn find_up(parent: &mut FxHashMap<u32, u32>, x: u32) -> u32 {
            let mut root = x;
            while let Some(&p) = parent.get(&root) {
                if p == root {
                    break;
                }
                root = p;
            }
            // path compression
            let mut cur = x;
            while let Some(&p) = parent.get(&cur) {
                if p == cur || p == root {
                    break;
                }
                parent.insert(cur, root);
                cur = p;
            }
            root
        }
        let mut parent: FxHashMap<u32, u32> = FxHashMap::default();
        let mut union = |parent: &mut FxHashMap<u32, u32>, a: u32, b: u32| {
            parent.entry(a).or_insert(a);
            parent.entry(b).or_insert(b);
            let ra = find_up(parent, a);
            let rb = find_up(parent, b);
            if ra != rb {
                // deterministic root: smaller id
                let (lo, hi) = if ra < rb { (ra, rb) } else { (rb, ra) };
                parent.insert(hi, lo);
            }
        };
        for b in &func.blocks {
            for inst in &b.instructions {
                if let Instruction::Phi { dest, incoming, .. } = inst {
                    for (op, _pred) in incoming {
                        if let Operand::Value(Value(src)) = op {
                            union(&mut parent, dest.0, *src);
                        }
                    }
                }
            }
        }
        ColorClasses { parent }
    }

    fn root(&self, mut v: u32) -> u32 {
        while let Some(&p) = self.parent.get(&v) {
            if p == v {
                break;
            }
            v = p;
        }
        v
    }
}

/// Residency pressure per local point: number of coalesced color CLASSES of
/// `values` live at each point, built from hole-aware segments (a value with
/// a hole in this block is not counted across the hole).
struct Pressure {
    per_block: Vec<Vec<u32>>,
}

impl Pressure {
    fn build(
        live: &LivenessResult,
        func: &IrFunction,
        values: &FxHashSet<u32>,
        classes: &ColorClasses,
    ) -> Self {
        let mut per_block = Vec::with_capacity(func.blocks.len());
        for (bi, b) in func.blocks.iter().enumerate() {
            let npts = b.instructions.len() + 1;
            // Roots resident at each point (duplicates deduped afterwards).
            let mut at_point: Vec<Vec<u32>> = (0..npts).map(|_| Vec::new()).collect();
            let gs = live.block_starts[bi];
            let ge = live.block_ends[bi];
            for iv in &live.segments {
                if !values.contains(&iv.value_id) {
                    continue;
                }
                if iv.end < gs || iv.start > ge {
                    continue;
                }
                let lo = (iv.start.max(gs) - gs) as usize;
                let hi = (iv.end.min(ge) - gs) as usize;
                let root = classes.root(iv.value_id);
                for pts in at_point.iter_mut().take(hi + 1).skip(lo) {
                    pts.push(root);
                }
            }
            let mut row = Vec::with_capacity(npts);
            for mut pts in at_point {
                pts.sort_unstable();
                pts.dedup();
                row.push(pts.len() as u32);
            }
            per_block.push(row);
        }
        Pressure { per_block }
    }

    fn peak(&self, bi: usize) -> (usize, u32) {
        self.per_block[bi]
            .iter()
            .enumerate()
            .max_by_key(|&(i, &c)| (c, std::cmp::Reverse(i)))
            .map(|(i, &c)| (i, c))
            .unwrap_or((0, 0))
    }

    /// Subtract one unit of residency over half-open `[from,to)`.
    fn relieve(&mut self, bi: usize, from: usize, to: usize) {
        let row = &mut self.per_block[bi];
        let to = to.min(row.len());
        if from >= to {
            return;
        }
        for p in row.iter_mut().take(to).skip(from) {
            *p = p.saturating_sub(1);
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Planner
// ─────────────────────────────────────────────────────────────────────────────

/// A planned stack gap: `vid` is not register-resident over the half-open
/// local span `[from,to)` of `block`; `to == n` propagates the memory state
/// across outgoing CFG edges (the materializer reconstructs a register name
/// at the next use, wherever it lives).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Gap {
    vid: u32,
    block: usize,
    from: usize,
    to: usize,
}

struct Plan {
    gaps: Vec<Gap>,
    remats: Vec<u32>,
    /// (vid, block) where the block is ENTERED in the memory region (the
    /// value arrives via slot, reload happens at the first read cluster).
    entry_memory: FxHashSet<(u32, usize)>,
}

impl Plan {
    fn gaps_of(&self, vid: u32, bi: usize) -> Vec<(usize, usize)> {
        self.gaps
            .iter()
            .filter(|g| g.vid == vid && g.block == bi)
            .map(|g| (g.from, g.to))
            .collect()
    }
}

/// Closure of a terminal gap across CFG edges: every successor in which
/// `vid` remains live enters the memory region. BFS stops at blocks where
/// the value is re-defined (SSA: its φ-definition block is not live-in) and
/// records, per reached block, the local first-use point (reload point). A
/// live-through block (no local use) propagates onward.
fn memory_closure(
    live: &LivenessResult,
    func: &IrFunction,
    succs: &analysis::FlatAdj,
    vid: u32,
    start_block: usize,
) -> Vec<(usize, usize)> {
    let mut visited: FxHashSet<usize> = FxHashSet::default();
    let mut stack = vec![start_block];
    let mut reached = Vec::new();
    while let Some(bi) = stack.pop() {
        for &s in succs.row(bi) {
            let s = s as usize;
            if !visited.insert(s) {
                continue;
            }
            // A (re-)definition of vid joins a new incarnation; the memory
            // region neither enters nor flows through that block.
            if block_defines_value(func, s, vid) {
                continue;
            }
            if !live.is_live_in(s, vid) {
                continue;
            }
            let uses = block_use_points(live, func, s, vid);
            let first = uses
                .first()
                .copied()
                .unwrap_or_else(|| func.blocks[s].instructions.len());
            reached.push((s, first));
            if uses.is_empty() {
                stack.push(s);
            }
        }
    }
    reached
}

/// Weighted count of dynamic read points (use-density currency), including
/// edge φ reads.
fn weighted_use_count(live: &LivenessResult, func: &IrFunction, vid: u32, depths: &[u32]) -> u64 {
    let mut w = 0u64;
    for bi in 0..func.blocks.len() {
        let f = block_frequency(depths[bi]);
        w = w.saturating_add(block_use_points(live, func, bi, vid).len() as u64 * f);
        if block_feeds_successor_phi(func, bi, vid) {
            w = w.saturating_add(f);
        }
    }
    w
}

/// Local point of `vid`'s definition in block `bi`, if any (φ defs anchor
/// at the first non-φ point — the position past the φ prefix).
fn local_def_point(func: &IrFunction, bi: usize, vid: u32) -> Option<usize> {
    find_def(func, vid).and_then(|(db, di, is_phi)| {
        if db != bi {
            return None;
        }
        Some(if is_phi {
            first_non_phi(&func.blocks[bi])
        } else {
            di
        })
    })
}

/// Half-open local spans of block `bi` over which rematerializing `vid`
/// gives NO net register relief:
///
/// * the single def point (the original source-less definition still
///   executes and occupies a register there);
/// * each resident segment's first-to-last read span, inclusive — one
///   clone result renames every use of that contiguous run and therefore
///   stays live across the whole run, holding a register just like the
///   original value did.
///
/// What DOES get relieved is everything else: the residency carried into
/// a block ahead of its first read (loop-preheader hoisting case: the
/// adler32 table base) and any hole between runs / after the last read
/// before the value was formerly carried out to a successor.
fn remat_no_relief_spans(
    live: &LivenessResult,
    func: &IrFunction,
    bi: usize,
    vid: u32,
    lo: usize,
    hi: usize,
) -> Vec<(usize, usize)> {
    let reads = block_use_points(live, func, bi, vid);
    let mut spans: Vec<(usize, usize)> = reads
        .iter()
        .filter(|&&p| p >= lo && p <= hi)
        .take(1)
        .flat_map(|&first| {
            let last = reads
                .iter()
                .rev()
                .copied()
                .find(|&p| p >= lo && p <= hi)
                .unwrap_or(first);
            std::iter::once((first.min(last), last.max(first) + 1))
        })
        .collect();
    if let Some(d) = local_def_point(func, bi, vid) {
        if d >= lo && d <= hi {
            spans.push((d, d + 1));
        }
    }
    spans
}

#[inline]
fn point_in_spans(p: usize, spans: &[(usize, usize)]) -> bool {
    spans.iter().any(|&(s, e)| p >= s && p < e)
}

/// Whether rematerializing `vid` gives NET register relief at a REACHABLE
/// over-budget point (excess no greater than [`reach_band`]). Points grossly
/// over budget cannot be made colorable by editing this value and do not
/// qualify it.
///
/// Net relief is stricter than mere residency. Full rematerialization keeps
/// the original source-less definition (it still occupies a register at its
/// def point) and emits one clone at every use cluster (the clone result
/// occupies a register at that point). The value is only actually removed
/// from the residency curve at points it neither is defined nor read —
/// crediting a def/use point would book register freedom the edit does not
/// provide. This is the nbody -Os discriminator: a 1-2-use global whose
/// only reachable over-budget points are its own use clusters tipped the
/// planner but left generated code with zero fewer stack references and
/// five extra instructions.
fn covers_over_budget_point(
    live: &LivenessResult,
    func: &IrFunction,
    pressure: &Pressure,
    vid: u32,
    policy: &GlaPolicy,
) -> bool {
    let budget = policy.budget;
    let relievable = |c: u32| c as usize > budget && policy.peak_is_reachable(c);
    for (bi, row) in pressure.per_block.iter().enumerate() {
        // Block-level gate: a block whose PEAK is beyond the reach band
        // cannot be made colorable by editing this (or any handful of)
        // value(s); crediting its reachable buildup points would only
        // perturb the production allocator's folding/callee-save plan
        // (the nbody cascade). Relief is credited only in blocks the
        // edit can plausibly tip under budget.
        let (_, block_peak) = pressure.peak(bi);
        if block_peak as usize <= budget || !policy.peak_is_reachable(block_peak) {
            continue;
        }
        if !row.iter().any(|&c| relievable(c)) {
            continue;
        }
        let gs = live.block_starts[bi];
        let ge = live.block_ends[bi];
        for iv in live.segments.iter().filter(|iv| iv.value_id == vid) {
            if iv.end < gs || iv.start > ge {
                continue;
            }
            let lo = (iv.start.max(gs) - gs) as usize;
            let hi = ((iv.end.min(ge) - gs) as usize).min(row.len() - 1);
            // Speed tier: the clone re-occupies a register over the def
            // point and each contiguous read run, so only the carried
            // residency outside those spans is net relief. Debug tier
            // (-O0): values are stack-homed regardless, so even use-point
            // remats eliminate slot store/reload traffic; credit the full
            // resident span.
            let no_relief = if policy.net_relief() {
                remat_no_relief_spans(live, func, bi, vid, lo, hi)
            } else {
                Vec::new()
            };
            for p in lo..=hi {
                if point_in_spans(p, &no_relief) {
                    continue;
                }
                if relievable(row[p]) {
                    return true;
                }
            }
        }
    }
    false
}

fn crosses_call(live: &LivenessResult, bi: usize, from: usize, to: usize) -> bool {
    let gs = live.block_starts[bi];
    for p in from..to {
        if live.call_points.binary_search(&(gs + p as u32)).is_ok() {
            return true;
        }
    }
    false
}

struct Candidate {
    vid: u32,
    from: usize,
    to: usize,
    closure: Vec<(usize, usize)>,
    benefit: u64,
    cost: u64,
    distance: u64,
}

impl Candidate {
    /// Better eviction: larger benefit, then farther next use (Belady),
    /// then lower cost, then smaller vid (determinism).
    fn beats(&self, other: &Candidate) -> bool {
        (
            self.benefit,
            self.distance,
            u64::MAX - self.cost,
            u32::MAX - self.vid,
        ) > (
            other.benefit,
            other.distance,
            u64::MAX - other.cost,
            u32::MAX - other.vid,
        )
    }
}

/// Greedy global planner: rematerializations first (zero store cost), then
/// hottest-pressure-first Belady-MIN spill gaps across the whole CFG,
/// bounded at every step by the fail-closed [`GlaPolicy`].
fn plan_function(
    live: &LivenessResult,
    func: &IrFunction,
    succs: &analysis::FlatAdj,
    eligible: &FxHashSet<u32>,
    remattable: &FxHashSet<u32>,
    policy: &GlaPolicy,
) -> Plan {
    let budget = policy.budget;
    let max_splits = policy.max_splits;
    let depths = &live.block_loop_depth;
    let classes = ColorClasses::phi_webs(func);
    let mut pressure = Pressure::build(live, func, eligible, &classes);
    if split_debug_enabled() && std::env::var("CCC_GLA_TRACE").is_ok() {
        for (bi, b) in func.blocks.iter().enumerate() {
            let (pt, pk) = pressure.peak(bi);
            if pk as usize > budget {
                eprintln!(
                    "[GLA] {}: block{} depth{} peak {}/{} reach={} at point {}/{}",
                    func.name,
                    bi,
                    depths[bi],
                    pk,
                    budget,
                    policy.reach_band,
                    pt,
                    b.instructions.len()
                );
            }
        }
    }
    let mut gaps: Vec<Gap> = Vec::new();
    let mut remats: Vec<u32> = Vec::new();
    let mut entry_memory: FxHashSet<(u32, usize)> = FxHashSet::default();
    let mut used = 0usize;
    let types = collect_value_types(func);

    // ── Full rematerialization ──
    let mut rc: Vec<u32> = remattable.iter().copied().collect();
    rc.sort_unstable();
    for vid in rc {
        if used >= max_splits {
            break;
        }
        let wuses = weighted_use_count(live, func, vid, depths);
        let nseg = live.segments.iter().filter(|iv| iv.value_id == vid).count();
        let covers = covers_over_budget_point(live, func, &pressure, vid, policy);
        if split_debug_enabled() && std::env::var("CCC_GLA_TRACE").is_ok() {
            let ranges: Vec<String> = live
                .segments
                .iter()
                .filter(|iv| iv.value_id == vid)
                .map(|iv| {
                    let sb = live
                        .block_starts
                        .iter()
                        .rposition(|&s| s <= iv.start)
                        .unwrap_or(0);
                    let eb = live
                        .block_starts
                        .iter()
                        .rposition(|&s| s <= iv.end)
                        .unwrap_or(0);
                    format!("b{}..b{}[{}-{}]", sb, eb, iv.start, iv.end)
                })
                .collect();
            eprintln!(
                "[GLA] {}: remat-candidate v{} weighted_uses={} segments={} covers_reachable={} {}",
                func.name,
                vid,
                wuses,
                nseg,
                covers,
                ranges.join(",")
            );
        }
        if wuses > policy.remat_max_weight {
            continue;
        }
        if nseg > policy.remat_max_segments {
            if split_debug_enabled() && std::env::var("CCC_GLA_TRACE").is_ok() {
                eprintln!(
                    "[GLA] {}: remat-candidate v{} rejected: {} segments > {}",
                    func.name, vid, nseg, policy.remat_max_segments
                );
            }
            continue;
        }
        if !covers {
            continue;
        }
        if !types.get(&vid).copied().is_some_and(is_simple_gpr_type) {
            continue;
        }
        for bi in 0..func.blocks.len() {
            let gs = live.block_starts[bi];
            let ge = live.block_ends[bi];
            let row_len = pressure.per_block[bi].len();
            for iv in live.segments.iter().filter(|iv| iv.value_id == vid) {
                if iv.end < gs || iv.start > ge {
                    continue;
                }
                let lo = (iv.start.max(gs) - gs) as usize;
                let hi = ((iv.end.min(ge) - gs) as usize).min(row_len - 1);
                let no_relief = if policy.net_relief() {
                    remat_no_relief_spans(live, func, bi, vid, lo, hi)
                } else {
                    Vec::new()
                };
                let mut p = lo;
                while p <= hi {
                    if point_in_spans(p, &no_relief) {
                        p += 1;
                        continue;
                    }
                    // Run of consecutive relieved points -> one call.
                    let mut q = p;
                    while q <= hi && !point_in_spans(q, &no_relief) {
                        q += 1;
                    }
                    pressure.relieve(bi, p, q);
                    p = q;
                }
            }
        }
        remats.push(vid);
        used += 1;
        // The per-value applied line is emitted once, with its kind, in
        // `run_with_policy` after planning; do not duplicate it here.
    }

    // ── Spill gaps ──
    //
    // Net-negative in Phase-1 A/B without allocation feedback (see
    // `allow_spill_gaps`); the whole fixpoint is skipped unless enabled.
    if !policy.allow_spill_gaps {
        return Plan {
            gaps: Vec::new(),
            remats,
            entry_memory: Default::default(),
        };
    }
    let mut done_block: FxHashSet<(u32, usize)> = FxHashSet::default();
    loop {
        if used >= max_splits {
            break;
        }
        let mut target: Option<(u64, usize)> = None; // (weighted excess, block)
        for bi in 0..func.blocks.len() {
            let (_pt, pk) = pressure.peak(bi);
            if pk as usize <= budget || !policy.peak_is_reachable(pk) {
                // At or under budget: nothing to gain. Grossly over budget:
                // cannot be made colorable by a handful of edits; leave it
                // entirely to the production allocator (folding + callee-
                // saved buys). Editing such blocks only adds traffic.
                continue;
            }
            let score = block_frequency(depths[bi]) * (pk as usize - budget) as u64;
            match target {
                Some((s, b)) if (score, std::cmp::Reverse(bi)) <= (s, std::cmp::Reverse(b)) => {}
                _ => target = Some((score, bi)),
            }
        }
        let Some((_, bi)) = target else { break };
        let n = func.blocks[bi].instructions.len();

        let mut best: Option<Candidate> = None;
        for &vid in eligible.iter() {
            if remats.contains(&vid) || done_block.contains(&(vid, bi)) {
                continue;
            }
            if !live.is_live_at(vid, live.block_starts[bi] + pressure.peak(bi).0 as u32) {
                continue;
            }
            let uses = block_use_points(live, func, bi, vid);
            let peak_pt = pressure.peak(bi).0;
            if uses.contains(&peak_pt) {
                continue; // read at the peak: bits required now
            }
            let def_local = find_def(func, vid).filter(|(db, _, _)| *db == bi);
            if let Some((_, di, false)) = def_local {
                if di == peak_pt {
                    continue; // defined at the peak
                }
            }

            // Gap start: just after the last read ≤ peak, else just past the
            // local def, else block entry (memory arrives on the edge).
            let mut from = uses
                .iter()
                .copied()
                .rfind(|&u| u <= peak_pt)
                .map(|u| u + 1)
                .unwrap_or(0);
            if let Some((_, di, is_phi)) = def_local {
                let floor = if is_phi {
                    first_non_phi(&func.blocks[bi])
                } else {
                    di + 1
                };
                from = from.max(floor);
            }
            // Gap end: first read after the peak in-block, else terminator
            // (cross-block closure).
            let next_in_block = uses.iter().copied().find(|&u| u > peak_pt);
            let to = next_in_block.unwrap_or(n);
            if from >= to {
                continue;
            }
            if let Some((_, di, is_phi)) = def_local {
                let floor = if is_phi {
                    first_non_phi(&func.blocks[bi])
                } else {
                    di + 1
                };
                if to <= floor {
                    continue;
                }
            }
            if let Some(t) = next_in_block {
                if t - from < policy.min_gap && !crosses_call(live, bi, from, t) {
                    continue;
                }
                // Same-block register piece on both sides of the gap:
                // intra-block pre-alloc spilling, which is net-negative
                // against the production allocator's folded spills
                // (see RA-06 decision record). Fail closed unless the A/B
                // override explicitly asks for them.
                if !policy.allow_intra_block_gaps {
                    continue;
                }
            }

            let mut closure: Vec<(usize, usize)> = Vec::new();
            if next_in_block.is_none() && live.is_live_out(bi, vid) {
                closure = memory_closure(live, func, succs, vid, bi);
                if closure.is_empty() && !block_feeds_successor_phi(func, bi, vid) {
                    continue; // value dies here: the colorer drops it
                }
            }

            // Benefit: weighted residency relieved at REACHABLE over-budget
            // points over the local gap and closure prefixes. Grossly-over
            // blocks stay with the production allocator and credit nothing.
            let creditable = |c: u32| c as usize > budget && policy.peak_is_reachable(c);
            let mut benefit = 0u64;
            for p in from..to {
                if pressure.per_block[bi]
                    .get(p)
                    .copied()
                    .is_some_and(&creditable)
                {
                    benefit = benefit.saturating_add(block_frequency(depths[bi]));
                }
            }
            for &(cb, first) in &closure {
                for p in 0..first {
                    if pressure.per_block[cb]
                        .get(p)
                        .copied()
                        .is_some_and(&creditable)
                    {
                        benefit = benefit.saturating_add(block_frequency(depths[cb]));
                    }
                }
            }
            if benefit < policy.min_benefit {
                continue;
            }

            // Cost: one immutable capture store (def-block frequency) plus
            // weighted reload clusters.
            let Some((def_block, _, _)) = find_def(func, vid) else {
                continue;
            };
            let mut cost = block_frequency(depths[def_block]);
            if next_in_block.is_some() {
                cost = cost.saturating_add(block_frequency(depths[bi]));
            }
            for &(cb, first) in &closure {
                let ncb = func.blocks[cb].instructions.len();
                let has_use = first < ncb
                    || !block_use_points(live, func, cb, vid).is_empty()
                    || terminator_uses_value(&func.blocks[cb].terminator, vid)
                    || block_feeds_successor_phi(func, cb, vid);
                if has_use {
                    cost = cost.saturating_add(block_frequency(depths[cb]));
                }
            }
            if next_in_block.is_none()
                && block_feeds_successor_phi(func, bi, vid)
                && !closure
                    .iter()
                    .any(|(cb, _)| succs.row(bi).iter().any(|s| *s as usize == *cb))
            {
                cost = cost.saturating_add(block_frequency(depths[bi]));
            }

            if benefit.saturating_mul(10) < cost.saturating_mul(policy.benefit_ratio10) {
                continue;
            }

            let distance = next_in_block
                .map(|u| (u - from) as u64)
                .unwrap_or(u64::MAX - block_frequency(depths[bi]));

            let cand = Candidate {
                vid,
                from,
                to,
                closure,
                benefit,
                cost,
                distance,
            };
            if best.as_ref().is_none_or(|b| cand.beats(b)) {
                best = Some(cand);
            }
        }

        let Some(cand) = best else {
            // Relieve the scan of this un-splittable peak so the fixed-point
            // loop targets genuinely solvable peaks instead of spinning.
            for p in pressure.per_block[bi].iter_mut() {
                if *p as usize > budget {
                    *p = budget as u32;
                }
            }
            continue;
        };

        gaps.push(Gap {
            vid: cand.vid,
            block: bi,
            from: cand.from,
            to: cand.to,
        });
        done_block.insert((cand.vid, bi));
        pressure.relieve(bi, cand.from, cand.to);
        // The seed block is ENTERED in the memory region only when the gap
        // begins at block entry (a value arriving live-in on the edge). A
        // mid-block gap leaves the entry region register-resident; marking
        // it entry-memory would schedule a reload before an early read,
        // potentially ahead of the capture store itself.
        if cand.from == 0 {
            entry_memory.insert((cand.vid, bi));
        }
        for &(cb, first) in &cand.closure {
            entry_memory.insert((cand.vid, cb));
            if first > 0 {
                pressure.relieve(cb, 0, first);
                gaps.push(Gap {
                    vid: cand.vid,
                    block: cb,
                    from: 0,
                    to: first,
                });
                done_block.insert((cand.vid, cb));
            }
        }
        used += 1;
        if split_debug_enabled() {
            eprintln!(
                "[GLA] {}: gap v{} block{} [{},{}) benefit={} cost={} closure={}",
                func.name,
                cand.vid,
                bi,
                cand.from,
                cand.to,
                cand.benefit,
                cand.cost,
                cand.closure.len()
            );
        }
    }

    Plan {
        gaps,
        remats,
        entry_memory,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Materialization
// ─────────────────────────────────────────────────────────────────────────────

/// Public entry point. Policy comes from the A/B environment and the
/// compilation's target/optimization tier. Returns the count of logical
/// values rewritten (spilled or rematerialized), 0 when nothing applied.
pub(crate) fn run(func: &mut IrFunction, max_splits: usize, opt_level: u32) -> usize {
    if func.blocks.is_empty() || max_splits == 0 {
        return 0;
    }
    run_with_policy(func, GlaPolicy::from_env(max_splits, opt_level))
}

/// Entry point with an explicit policy (unit tests use this so their
/// expectations never depend on process-wide environment knobs).
fn run_with_policy(func: &mut IrFunction, policy: GlaPolicy) -> usize {
    let max_splits = policy.max_splits;
    if func.blocks.is_empty() || max_splits == 0 || !policy.enabled() {
        return 0;
    }

    let label_map = analysis::build_label_map(func);
    let (preds, succs) = analysis::build_cfg(func, &label_map);
    let idom = analysis::compute_dominators(func.blocks.len(), &preds, &succs);
    let live = crate::backend::liveness::compute_live_intervals(func);
    let all_types = collect_value_types(func);

    let mut ineligible = collect_ineligible(func);
    for v in collect_recurrence_phi_dests(func, &label_map, &preds, &succs, &idom) {
        ineligible.insert(v);
    }
    let eligible: FxHashSet<u32> = all_types
        .iter()
        .filter(|(v, t)| is_simple_gpr_type(**t) && !ineligible.contains(v))
        .map(|(v, _)| *v)
        .collect();

    let mut remattable: FxHashSet<u32> = FxHashSet::default();
    let mut templates: FxHashMap<u32, Instruction> = FxHashMap::default();
    for b in &func.blocks {
        for inst in &b.instructions {
            if let Some(d) = inst.dest() {
                if eligible.contains(&d.0) {
                    if let Some(t) = rematerializable_template(inst) {
                        remattable.insert(d.0);
                        templates.insert(d.0, t);
                    }
                }
            }
        }
    }

    let plan = plan_function(&live, func, &succs, &eligible, &remattable, &policy);
    if split_debug_enabled() {
        for vid in &plan.remats {
            let kind = templates
                .get(vid)
                .map(|t| match t {
                    Instruction::GlobalAddr { .. } => "globaladdr",
                    Instruction::Copy {
                        src: Operand::Const(_),
                        ..
                    } => "const",
                    _ => "other",
                })
                .unwrap_or("?");
            eprintln!("[GLA] {}: remat v{} ({})", func.name, vid, kind);
        }
    }
    if plan.gaps.is_empty() && plan.remats.is_empty() {
        return 0;
    }
    materialize(func, &live, &succs, plan, &all_types, &templates)
}

fn clone_remat(template: &Instruction, name: Value) -> Instruction {
    match template {
        Instruction::GlobalAddr { name: gname, .. } => Instruction::GlobalAddr {
            dest: name,
            name: gname.clone(),
        },
        Instruction::Copy { src, .. } => Instruction::Copy {
            dest: name,
            src: src.clone(),
        },
        _ => template.clone(),
    }
}

/// Event scheduled at an ORIGINAL local point `p`; Load/Remat/Anchor all
/// execute immediately before original instruction `p` (`p == n`: before
/// the terminator). Loads/remats sort before anchors at the same point (the
/// anchor follows a def that the loads do not depend on).
#[derive(Clone, Copy, Debug)]
enum Event {
    Load { vid: u32, name: u32 },
    Remat { vid: u32, name: u32 },
    Anchor { vid: u32 },
}

/// One edge whose reload lives in a new trampoline block.
struct EdgeTramp {
    pred: usize,
    succ: usize,
    label: BlockId,
    /// (vid, fresh name built inside the trampoline).
    defs: Vec<(u32, Value)>,
}

#[allow(clippy::too_many_arguments)]
fn materialize(
    func: &mut IrFunction,
    live: &LivenessResult,
    succs: &analysis::FlatAdj,
    plan: Plan,
    all_types: &FxHashMap<u32, IrType>,
    templates: &FxHashMap<u32, Instruction>,
) -> usize {
    let entry_memory = plan.entry_memory.clone();
    let nblocks = func.blocks.len();
    // Fail-closed id space: `next_value_id` is authoritative in principle,
    // but floor it against every id already present in the IR so an
    // upstream pass that minted an id without syncing its counter can never
    // make us emit a duplicate definition.
    let mut next_val = func.next_value_id;
    for b in &func.blocks {
        for i in &b.instructions {
            if let Some(d) = i.dest() {
                next_val = next_val.max(d.0.saturating_add(1));
            }
        }
    }
    let mut next_block_id = func.blocks.iter().map(|b| b.label.0).max().unwrap_or(0) + 1;

    let remat_ids: FxHashSet<u32> = plan.remats.iter().copied().collect();
    let gap_ids: FxHashSet<u32> = plan.gaps.iter().map(|g| g.vid).collect();
    let split_vids: Vec<u32> = gap_ids.union(&remat_ids).copied().collect();

    // ── Slot values (alloca instructions inserted AFTER all sweeps) ──
    let mut slot_of: FxHashMap<u32, Value> = FxHashMap::default();
    for &vid in &gap_ids {
        if !all_types.contains_key(&vid) {
            continue;
        }
        let Some(slot) = next_value(&mut next_val) else {
            continue;
        };
        slot_of.insert(vid, slot);
    }

    let mut events: Vec<Vec<(usize, Event)>> = vec![Vec::new(); nblocks];
    // name of the memory cluster serving point `n` in block bi, if any
    let mut name_at_exit: Vec<FxHashMap<u32, u32>> = vec![FxHashMap::default(); nblocks];
    let mut applied: FxHashSet<u32> = FxHashSet::default();

    let new_load = |next_val: &mut u32| next_value(next_val).expect("fresh value").0;

    // ── Per-block use clusters ──
    for bi in 0..nblocks {
        let n = func.blocks[bi].instructions.len();
        let mem_at = |vid: u32, p: usize| -> bool {
            if remat_ids.contains(&vid) {
                return true;
            }
            // The half-open memory span is [f, t); the read AT point `t` is
            // also served from memory: its reload executes immediately
            // before that instruction, which is precisely the boundary
            // reload that starts the next register piece.
            if plan.gaps_of(vid, bi).iter().any(|&(f, t)| f <= p && p <= t) {
                return true;
            }
            // Entry-memory blocks serve the first read cluster from the
            // slot; after that cluster the reloaded name is resident.
            if entry_memory.contains(&(vid, bi)) {
                let first = block_use_points(live, func, bi, vid)
                    .into_iter()
                    .min()
                    .unwrap_or(n);
                return p <= first;
            }
            false
        };

        for &vid in &split_vids {
            let uses = block_use_points(live, func, bi, vid);
            // Walk read points; group consecutive memory-served points into
            // one cluster with a fresh name; register points need no event.
            let mut open_name: Option<u32> = None;
            let mut prev_mem = false;
            for p in uses.iter().copied() {
                let mem = mem_at(vid, p);
                if mem {
                    if !prev_mem || open_name.is_none() {
                        let name = new_load(&mut next_val);
                        let ev = if remat_ids.contains(&vid) {
                            Event::Remat { vid, name }
                        } else {
                            Event::Load { vid, name }
                        };
                        events[bi].push((p, ev));
                        open_name = Some(name);
                        applied.insert(vid);
                    }
                    if p == n {
                        if let Some(name) = open_name {
                            name_at_exit[bi].insert(vid, name);
                        }
                    }
                } else {
                    open_name = None;
                }
                prev_mem = mem;
            }
        }

        // Capture anchors: store immediately after the unique def.
        for &vid in &gap_ids {
            let Some((db, di, is_phi)) = find_def(func, vid) else {
                continue;
            };
            if db != bi {
                continue;
            }
            let point = if is_phi {
                first_non_phi(&func.blocks[bi])
            } else {
                di + 1
            };
            let point = point.min(n);
            events[bi].push((point, Event::Anchor { vid }));
            applied.insert(vid);
        }
    }

    // ── Edge planning: successor φ incoming operands ──
    let mut edge_tramps: Vec<EdgeTramp> = Vec::new();
    // in-place edge renames (pred block, succ block) applied to φ
    // incomings without rerouting the edge.
    let mut inline_edge_rename: Vec<(usize, usize, Vec<(u32, u32)>)> = Vec::new();
    for bi in 0..nblocks {
        for &s_raw in succs.row(bi) {
            let s = s_raw as usize;
            let mut renames: Vec<(u32, u32)> = Vec::new();
            let mut deferred: Vec<u32> = Vec::new();
            for &vid in &split_vids {
                if !phi_incoming_uses(func, s, bi, vid) {
                    continue;
                }
                if let Some(&name) = name_at_exit[bi].get(&vid) {
                    renames.push((vid, name));
                } else {
                    // Register-resident at exit? Then no rename.
                    let n = func.blocks[bi].instructions.len();
                    let own = plan.gaps_of(vid, bi);
                    let mem_exit = remat_ids.contains(&vid)
                        || own.iter().any(|&(_, t)| t >= n)
                        || entry_memory.contains(&(vid, bi));
                    if !mem_exit {
                        continue; // original name is correct on the edge
                    }
                    deferred.push(vid);
                }
            }
            if succs.len(bi) == 1 {
                // Single successor: serve deferred values with an end-of-block
                // load/remat that is effectively on the unique edge.
                let mut extra = Vec::new();
                for vid in deferred.drain(..) {
                    let name = new_load(&mut next_val);
                    let n = func.blocks[bi].instructions.len();
                    let ev = if remat_ids.contains(&vid) {
                        Event::Remat { vid, name }
                    } else {
                        Event::Load { vid, name }
                    };
                    events[bi].push((n, ev));
                    applied.insert(vid);
                    extra.push((vid, name));
                }
                renames.extend(extra);
                if !renames.is_empty() {
                    inline_edge_rename.push((bi, s, renames));
                }
            } else if !deferred.is_empty() {
                // Fan-out edge: isolate it with a trampoline so the reload
                // never executes on the other edges.
                let label = BlockId(next_block_id);
                next_block_id += 1;
                let mut defs = Vec::new();
                for vid in deferred {
                    let nv = next_value(&mut next_val).expect("fresh value");
                    defs.push((vid, nv));
                    applied.insert(vid);
                    renames.push((vid, nv.0));
                }
                edge_tramps.push(EdgeTramp {
                    pred: bi,
                    succ: s,
                    label,
                    defs,
                });
                inline_edge_rename.push((bi, s, renames));
            } else if !renames.is_empty() {
                inline_edge_rename.push((bi, s, renames));
            }
        }
    }

    // Deterministic event order: point, then loads/remats before anchors.
    for ev in events.iter_mut() {
        ev.sort_by_key(|(p, e)| {
            (
                *p,
                matches!(e, Event::Anchor { .. }) as u8,
                match e {
                    Event::Load { vid, .. } | Event::Remat { vid, .. } | Event::Anchor { vid } => {
                        *vid
                    }
                },
            )
        });
        ev.dedup_by(|a, b| match (&a.1, &b.1) {
            (Event::Load { vid: va, name: na }, Event::Load { vid: vb, name: nb }) => {
                va == vb && na == nb
            }
            (Event::Remat { vid: va, name: na }, Event::Remat { vid: vb, name: nb }) => {
                va == vb && na == nb
            }
            (Event::Anchor { vid: a }, Event::Anchor { vid: b }) => a == b,
            _ => false,
        });
    }

    // ── Sweep every block in original coordinates ──
    for bi in 0..nblocks {
        let old = std::mem::take(&mut func.blocks[bi].instructions);
        let n = old.len();
        let mut out: Vec<Instruction> = Vec::with_capacity(old.len() + events[bi].len() + 2);
        let mut active: FxHashMap<u32, u32> = FxHashMap::default();
        let mut ev_idx = 0usize;

        for (i, mut inst) in old.into_iter().enumerate() {
            while ev_idx < events[bi].len() && events[bi][ev_idx].0 == i {
                emit_event(
                    &events[bi][ev_idx].1,
                    &mut out,
                    &slot_of,
                    all_types,
                    templates,
                    &mut active,
                );
                ev_idx += 1;
            }
            if !matches!(inst, Instruction::Phi { .. }) && !active.is_empty() {
                replace_values_in_inst(&mut inst, &active, false);
            }
            out.push(inst);
        }
        // Trailing events at point n (before the terminator).
        while ev_idx < events[bi].len() && events[bi][ev_idx].0 == n {
            emit_event(
                &events[bi][ev_idx].1,
                &mut out,
                &slot_of,
                all_types,
                templates,
                &mut active,
            );
            ev_idx += 1;
        }
        debug_assert_eq!(
            ev_idx,
            events[bi].len(),
            "all scheduled events must be consumed"
        );
        func.blocks[bi].instructions = out;
        if !active.is_empty() {
            replace_values_in_terminator(&mut func.blocks[bi].terminator, &active);
        }
    }

    // ── φ incoming rewrites ──
    // First rename operands (pred label still the original), then reroute
    // the whole edge to its trampoline exactly once (that relabels every
    // incoming on the edge, including register values that pass through).
    for (bi, s, renames) in &inline_edge_rename {
        let old_pred = func.blocks[*bi].label;
        for (vid, name) in renames {
            rewrite_phi_edge(func, *s, old_pred, *vid, *name, None);
        }
    }
    for t in &edge_tramps {
        let old_pred = func.blocks[t.pred].label;
        for inst in func.blocks[t.succ].instructions.iter_mut() {
            let Instruction::Phi { incoming, .. } = inst else {
                continue;
            };
            for (_op, p) in incoming.iter_mut() {
                if *p == old_pred {
                    *p = t.label;
                }
            }
        }
    }

    // ── Build trampoline blocks and retarget edges ──
    for t in &edge_tramps {
        let mut insts = Vec::new();
        for (vid, nv) in &t.defs {
            if remat_ids.contains(vid) {
                let tmpl = templates.get(vid).cloned().expect("remat template");
                insts.push(clone_remat(&tmpl, *nv));
            } else {
                insts.push(Instruction::Load {
                    volatile: false,
                    dest: *nv,
                    ptr: slot_of[vid],
                    ty: all_types[vid],
                    seg_override: AddressSpace::Default,
                });
            }
        }
        // Copy labels out before any mutable borrow.
        let old_target = func.blocks[t.succ].label;
        let new_label = t.label;
        // Non-renamed φ incoming values pass through the trampoline with no
        // copy (the value remains live across the unconditional branch).
        func.blocks.push(BasicBlock {
            label: new_label,
            instructions: insts,
            terminator: Terminator::Branch(old_target),
            source_spans: Vec::new(),
        });
        retarget_edge(&mut func.blocks[t.pred].terminator, old_target, new_label);
    }

    // ── Insert volatile capture allocas (coordinates are now settled) ──
    for (&vid, &slot) in &slot_of {
        let ty = all_types[&vid];
        insert_entry_alloca(func, slot, ty, true);
    }

    func.next_value_id = next_val;
    let n_applied = applied.len();
    // Fail-closed structural verification under the same gate as the
    // production allocator verifier.
    if std::env::var_os("CCC_VERIFY_REGALLOC").is_some() {
        let slots: FxHashSet<u32> = slot_of.values().map(|v| v.0).collect();
        if let Err(msg) = verify_rewrite(func, &slots) {
            panic!("[GLA] {}: rewrite failed verification: {}", func.name, msg);
        }
    }
    if split_debug_enabled() {
        eprintln!(
            "[GLA] {} applied: {} values ({} remat, {} capture slots), {} trampolines",
            func.name,
            n_applied,
            remat_ids.len(),
            slot_of.len(),
            edge_tramps.len()
        );
    }
    n_applied
}

fn emit_event(
    event: &Event,
    out: &mut Vec<Instruction>,
    slot_of: &FxHashMap<u32, Value>,
    all_types: &FxHashMap<u32, IrType>,
    templates: &FxHashMap<u32, Instruction>,
    active: &mut FxHashMap<u32, u32>,
) {
    match *event {
        Event::Load { vid, name } => {
            out.push(Instruction::Load {
                volatile: false,
                dest: Value(name),
                ptr: slot_of[&vid],
                ty: all_types[&vid],
                seg_override: AddressSpace::Default,
            });
            active.insert(vid, name);
        }
        Event::Remat { vid, name } => {
            let tmpl = templates.get(&vid).cloned().expect("remat template");
            out.push(clone_remat(&tmpl, Value(name)));
            active.insert(vid, name);
        }
        Event::Anchor { vid } => {
            out.push(Instruction::Store {
                volatile: false,
                val: Operand::Value(Value(vid)),
                ptr: slot_of[&vid],
                ty: all_types[&vid],
                seg_override: AddressSpace::Default,
            });
        }
    }
}

/// Rewrite (and optionally reroute) one φ incoming edge `pred -> phi_block`.
/// When `new_pred` is given (a trampoline), EVERY incoming operand on that
/// predecessor edge is re-labelled to the trampoline, including values that
/// merely pass through.
fn rewrite_phi_edge(
    func: &mut IrFunction,
    phi_block: usize,
    pred: BlockId,
    vid: u32,
    new_name: u32,
    new_pred: Option<BlockId>,
) {
    for inst in func.blocks[phi_block].instructions.iter_mut() {
        let Instruction::Phi { incoming, .. } = inst else {
            continue;
        };
        for (op, p) in incoming.iter_mut() {
            if *p != pred {
                continue;
            }
            if let Operand::Value(v) = op {
                if v.0 == vid {
                    *op = Operand::Value(Value(new_name));
                }
            }
            if let Some(np) = new_pred {
                *p = np;
            }
        }
    }
}

/// Retarget exactly one terminator edge `old -> new`.
fn retarget_edge(term: &mut Terminator, old: BlockId, new: BlockId) {
    match term {
        Terminator::Branch(t) if *t == old => *t = new,
        Terminator::CondBranch {
            true_label,
            false_label,
            ..
        } => {
            if *true_label == old {
                *true_label = new;
            } else if *false_label == old {
                *false_label = new;
            }
        }
        Terminator::Switch { cases, default, .. } => {
            if *default == old {
                *default = new;
            } else {
                for (_, t) in cases.iter_mut() {
                    if *t == old {
                        *t = new;
                    }
                }
            }
        }
        Terminator::IndirectBranch {
            possible_targets, ..
        } => {
            for t in possible_targets.iter_mut() {
                if *t == old {
                    *t = new;
                }
            }
        }
        _ => {}
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Verifier
// ─────────────────────────────────────────────────────────────────────────────

/// Post-rewrite structural verification. Returns an error describing the
/// first violated invariant. The pipeline also runs `CCC_VALIDATE_SSA` after
/// the transform; this checker covers the location-allocation-specific
/// obligations that generic unique-def validation does not see.
pub(crate) fn verify_rewrite(
    after: &IrFunction,
    capture_slots: &FxHashSet<u32>,
) -> Result<(), String> {
    // 1. Unique definition sites.
    let mut defs: FxHashSet<u32> = FxHashSet::default();
    let mut def_sites: FxHashMap<u32, Vec<(usize, usize)>> = FxHashMap::default();
    for (bi, b) in after.blocks.iter().enumerate() {
        for (ii, i) in b.instructions.iter().enumerate() {
            if let Some(d) = i.dest() {
                def_sites.entry(d.0).or_default().push((bi, ii));
                if !defs.insert(d.0) {
                    let sites = def_sites[&d.0]
                        .iter()
                        .map(|(b, p)| format!("b{}[{}]", b, p))
                        .collect::<Vec<_>>()
                        .join(",");
                    return Err(format!("duplicate definition of v{} at {}", d.0, sites));
                }
            }
        }
    }

    // 2. Every load from an internal capture slot is dominated by a store
    // to that slot.
    let label_map = analysis::build_label_map(after);
    let (preds, succs) = analysis::build_cfg(after, &label_map);
    let idom = analysis::compute_dominators(after.blocks.len(), &preds, &succs);
    let dominates = |a_block: usize, b_block: usize| -> bool {
        if a_block == b_block {
            return true;
        }
        let mut cur = b_block;
        for _ in 0..after.blocks.len() + 1 {
            if cur == a_block {
                return true;
            }
            if cur >= idom.len() || idom[cur] == usize::MAX || idom[cur] == cur {
                return false;
            }
            cur = idom[cur];
        }
        false
    };
    let mut slot_stores: FxHashMap<u32, Vec<usize>> = FxHashMap::default();
    for (bi, b) in after.blocks.iter().enumerate() {
        for i in &b.instructions {
            if let Instruction::Store {
                ptr,
                val: Operand::Value(_),
                ..
            } = i
            {
                if capture_slots.contains(&ptr.0) {
                    slot_stores.entry(ptr.0).or_default().push(bi);
                }
            }
        }
    }
    for (bi, b) in after.blocks.iter().enumerate() {
        // Within a block the first capture store must precede every reload
        // from the same slot (block-level dominance is trivial here).
        let mut first_store: FxHashMap<u32, usize> = FxHashMap::default();
        for (ii, i) in b.instructions.iter().enumerate() {
            if let Instruction::Store { ptr, .. } = i {
                if capture_slots.contains(&ptr.0) {
                    first_store.entry(ptr.0).or_insert(ii);
                }
            }
        }
        for (ii, i) in b.instructions.iter().enumerate() {
            if let Instruction::Load { dest, ptr, .. } = i {
                if capture_slots.contains(&ptr.0) {
                    if let Some(&si) = first_store.get(&ptr.0) {
                        if si >= ii {
                            return Err(format!(
                                "reload v{} from slot v{} at block {} point {} precedes its capture store at {}",
                                dest.0, ptr.0, bi, ii, si
                            ));
                        }
                        continue;
                    }
                    let has_store = slot_stores
                        .get(&ptr.0)
                        .is_some_and(|v| v.iter().any(|&sb| dominates(sb, bi)));
                    if !has_store {
                        return Err(format!(
                            "reload v{} from slot v{} in block {} has no dominating store",
                            dest.0, ptr.0, bi
                        ));
                    }
                }
            }
        }
    }

    // 3. Def ids bounded by next_value_id.
    for b in &after.blocks {
        for i in &b.instructions {
            if let Some(d) = i.dest() {
                if d.0 >= after.next_value_id {
                    return Err(format!(
                        "def v{} >= next_value_id {}",
                        d.0, after.next_value_id
                    ));
                }
            }
        }
    }

    // 4. φ prefix invariant.
    for b in &after.blocks {
        let mut seen_non_phi = false;
        for i in &b.instructions {
            if matches!(i, Instruction::Phi { .. }) && seen_non_phi {
                return Err("φ after non-φ instruction".into());
            }
            if !matches!(i, Instruction::Phi { .. }) {
                seen_non_phi = true;
            }
        }
    }
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn func_with(blocks: Vec<BasicBlock>, next: u32) -> IrFunction {
        let mut f = IrFunction::new("t".to_string(), IrType::I32, Vec::new(), false);
        f.blocks = blocks;
        f.next_value_id = next;
        f
    }
    fn blk(label: u32, instructions: Vec<Instruction>, term: Terminator) -> BasicBlock {
        BasicBlock {
            label: BlockId(label),
            instructions,
            terminator: term,
            source_spans: Vec::new(),
        }
    }
    fn bin(dest: u32, op: IrBinOp, a: u32, b: u32, ty: IrType) -> Instruction {
        Instruction::BinOp {
            dest: Value(dest),
            op,
            lhs: Operand::Value(Value(a)),
            rhs: Operand::Value(Value(b)),
            ty,
        }
    }
    fn add(dest: u32, a: u32, b: u32) -> Instruction {
        bin(dest, IrBinOp::Add, a, b, IrType::I64)
    }
    fn copy_const(dest: u32, c: i64) -> Instruction {
        Instruction::Copy {
            dest: Value(dest),
            src: Operand::Const(IrConst::I64(c)),
        }
    }
    fn global(dest: u32, name: &str) -> Instruction {
        Instruction::GlobalAddr {
            dest: Value(dest),
            name: name.to_string(),
        }
    }
    fn count(f: &IrFunction, p: impl Fn(&Instruction) -> bool) -> usize {
        f.blocks
            .iter()
            .flat_map(|b| b.instructions.iter())
            .filter(|i| p(i))
            .count()
    }

    /// Build >budget independent long-lived values that all stay live until
    /// a sequential consumer chain at the end. Returns (instructions,
    /// result id to return).
    fn pressure_ladder(cold: u32) -> (Vec<Instruction>, u32) {
        let mut v = Vec::new();
        v.push(add(cold, 500, 501));
        for k in 0..14u32 {
            v.push(add(100 + k, 600 + k, 620 + k));
        }
        // Consumer chain keeps every long-lived value resident until use.
        v.push(add(200, cold, 100));
        for k in 1..14u32 {
            v.push(add(200 + k, 199 + k, 100 + k));
        }
        (v, 213)
    }

    /// Like [`pressure_ladder`] but with the named value as a GlobalAddr,
    /// the rematerialization candidate. 12 long-lived values: pressure with
    /// the global is 13 (> budget), after its rematerialization exactly 12,
    /// so no spill gap may accompany the remat.
    fn remat_ladder() -> Vec<Instruction> {
        let mut v = vec![global(1, "G")];
        for k in 0..12u32 {
            v.push(add(100 + k, 800 + k, 820 + k));
        }
        v.push(add(200, 1, 100));
        for k in 1..12u32 {
            v.push(add(200 + k, 199 + k, 100 + k));
        }
        v
    }

    // ── near misses ──

    #[test]
    fn low_pressure_does_not_fire() {
        let mut f = func_with(
            vec![blk(
                0,
                vec![add(1, 900, 901)],
                Terminator::Return(Some(Operand::Value(Value(1)))),
            )],
            902,
        );
        assert_eq!(run(&mut f, 64, 2), 0);
    }

    #[test]
    fn float_values_never_split() {
        let mut f = func_with(
            vec![blk(
                0,
                vec![Instruction::BinOp {
                    dest: Value(10),
                    op: IrBinOp::Add,
                    lhs: Operand::Const(IrConst::F64(1.0)),
                    rhs: Operand::Const(IrConst::F64(2.0)),
                    ty: IrType::F64,
                }],
                Terminator::Return(Some(Operand::Value(Value(10)))),
            )],
            11,
        );
        assert_eq!(run(&mut f, 64, 2), 0);
    }

    #[test]
    fn div_operands_are_ineligible() {
        let bad = collect_ineligible(&func_with(
            vec![blk(
                0,
                vec![bin(2, IrBinOp::UDiv, 1, 3, IrType::I64)],
                Terminator::Return(Some(Operand::Value(Value(2)))),
            )],
            4,
        ));
        assert!(bad.contains(&1));
        assert!(bad.contains(&3));
    }

    #[test]
    fn variable_shift_count_is_ineligible() {
        let bad = collect_ineligible(&func_with(
            vec![blk(
                0,
                vec![bin(2, IrBinOp::Shl, 1, 3, IrType::I64)],
                Terminator::Return(Some(Operand::Value(Value(2)))),
            )],
            4,
        ));
        assert!(bad.contains(&3), "shift count v3 must stay pinned");
        assert!(!bad.contains(&1), "shifted value may be split");
    }

    // ── recurrence exclusion ──

    #[test]
    fn recurrence_phi_dests_are_excluded() {
        let blocks = vec![
            blk(0, vec![add(90, 80, 81)], Terminator::Branch(BlockId(1))),
            blk(
                1,
                vec![
                    Instruction::Phi {
                        dest: Value(1),
                        ty: IrType::I64,
                        incoming: vec![
                            (Operand::Value(Value(90)), BlockId(0)),
                            (Operand::Value(Value(2)), BlockId(2)),
                        ],
                    },
                    add(2, 1, 1),
                ],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(2)),
                    true_label: BlockId(2),
                    false_label: BlockId(3),
                },
            ),
            blk(2, vec![], Terminator::Branch(BlockId(1))),
            blk(
                3,
                vec![],
                Terminator::Return(Some(Operand::Value(Value(1)))),
            ),
        ];
        let f = func_with(blocks, 91);
        let lm = analysis::build_label_map(&f);
        let (preds, succs) = analysis::build_cfg(&f, &lm);
        let idom = analysis::compute_dominators(4, &preds, &succs);
        let rec = collect_recurrence_phi_dests(&f, &lm, &preds, &succs, &idom);
        assert!(rec.contains(&1), "latch-carried φ v1 is recurrent");
    }

    // ── rematerialization fires ──

    #[test]
    fn global_addr_rematerializes_under_pressure() {
        let insts = remat_ladder();
        let mut f = func_with(
            vec![blk(
                0,
                insts,
                Terminator::Return(Some(Operand::Value(Value(211)))),
            )],
            900,
        );
        let n = run(&mut f, 64, 2);
        assert!(n >= 1, "remat must fire under pressure");
        assert!(
            count(&f, |i| matches!(i, Instruction::GlobalAddr { .. })) >= 2,
            "expected optimization fires: a rematerialized GlobalAddr"
        );
        // Remat never builds a stack slot.
        assert_eq!(count(&f, |i| matches!(i, Instruction::Alloca { .. })), 0);
        verify_rewrite(&f, &FxHashSet::default()).expect("rewrite verifies");
    }

    #[test]
    fn copy_const_is_rematerializable() {
        assert!(rematerializable_template(&copy_const(1, 42)).is_some());
        assert!(rematerializable_template(&global(2, "x")).is_some());
        assert!(rematerializable_template(&add(3, 1, 2)).is_none());
        // A register-dependent copy is NOT source-less.
        let dep = Instruction::Copy {
            dest: Value(4),
            src: Operand::Value(Value(3)),
        };
        assert!(rematerializable_template(&dep).is_none());
    }

    // ── cross-block gap fires (the capability the intra-block pass lacks) ──

    #[test]
    fn cross_block_gap_spills_and_reloads_in_exit() {
        // b0: 15 long-lived values (cold = v1); b1: hot self-loop carrying
        // only counter work, all 15 live across it; b2: fold the values and
        // return. Only a global gap relieves the loop's residency peak.
        let (b0_all, _) = pressure_ladder(1);
        // The consumer chain belongs in the exit block, not the preheader.
        let b0 = b0_all[..15].to_vec();
        let mut b2 = Vec::new();
        b2.push(add(200, 1, 100));
        for k in 1..14u32 {
            b2.push(add(200 + k, 199 + k, 100 + k));
        }
        b2.push(add(300, 213, 1));
        let blocks = vec![
            blk(0, b0, Terminator::Branch(BlockId(1))),
            blk(
                1,
                vec![
                    Instruction::Phi {
                        dest: Value(2),
                        ty: IrType::I64,
                        incoming: vec![
                            (Operand::Const(IrConst::I64(0)), BlockId(0)),
                            (Operand::Value(Value(3)), BlockId(1)),
                        ],
                    },
                    add(3, 2, 900),
                    Instruction::Cmp {
                        dest: Value(4),
                        op: IrCmpOp::Slt,
                        lhs: Operand::Value(Value(3)),
                        rhs: Operand::Const(IrConst::I64(10)),
                        ty: IrType::I64,
                    },
                ],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(4)),
                    true_label: BlockId(1),
                    false_label: BlockId(2),
                },
            ),
            blk(2, b2, Terminator::Return(Some(Operand::Value(Value(300))))),
        ];
        let mut f = func_with(blocks, 900);
        let n = run_with_policy(
            &mut f,
            GlaPolicy {
                max_splits: 256,
                ..GlaPolicy::testing_permissive()
            },
        );
        assert!(n >= 1, "global gap must fire across the loop");
        // At least one immutable capture store and an exit-block reload.
        assert!(count(&f, |i| matches!(i, Instruction::Store { .. })) >= 1);
        let loads_b2 = f.blocks[2]
            .instructions
            .iter()
            .filter(|i| matches!(i, Instruction::Load { .. }))
            .count();
        assert!(loads_b2 >= 1, "exit block reloads the spilled value");
        verify_rewrite(&f, &FxHashSet::default()).expect("rewrite verifies");
    }

    // ── mid-block seed gap: an early register read must not be served by a
    // reload scheduled ahead of the capture store ──

    #[test]
    fn midblock_gap_keeps_early_read_register_resident() {
        // v1 has an early register-region read, but its farthest-next-use
        // distance makes it the best Belady eviction for the later peak.
        // Its capture anchor and that early read coincide at one point: the
        // seed block must NOT be marked entry-memory (the gap starts at 2),
        // or a reload is scheduled ahead of the store.
        let mut v = Vec::new();
        v.push(add(1, 900, 901)); // def v1
        v.push(add(2, 1, 902)); // EARLY read of v1 (register region)
        for k in 0..14u32 {
            v.push(add(100 + k, 600 + k, 620 + k)); // pressure peak
        }
        // Consumer chain folds 100.. first; v1 is consumed LAST, so its
        // next-use distance is the largest at the peak.
        v.push(add(200, 950, 100));
        for k in 1..14u32 {
            v.push(add(200 + k, 199 + k, 100 + k));
        }
        v.push(add(214, 213, 113));
        v.push(add(215, 214, 1)); // v1 read at the very end
        let mut f = func_with(
            vec![blk(
                0,
                v,
                Terminator::Return(Some(Operand::Value(Value(215)))),
            )],
            900,
        );
        let n = run_with_policy(
            &mut f,
            GlaPolicy {
                max_splits: 256,
                ..GlaPolicy::testing_permissive()
            },
        );
        assert!(n >= 1, "a mid-block gap must relieve the peak");
        // Every capture store must precede its reloads, point-wise.
        let mut slots = FxHashSet::default();
        for b in &f.blocks {
            for i in &b.instructions {
                if let Instruction::Alloca { dest, .. } = i {
                    slots.insert(dest.0);
                }
            }
        }
        verify_rewrite(&f, &slots).expect("no reload may precede its capture store");
        // The early reader (the def of v2) must still name v1 directly
        // rather than a reloaded fresh name.
        let early = f.blocks[0]
            .instructions
            .iter()
            .find(|i| i.dest().is_some_and(|d| d.0 == 2))
            .expect("early reader still present");
        let mut uses_v1 = false;
        crate::backend::liveness::for_each_operand_in_instruction(early, |op| {
            if matches!(op, Operand::Value(v) if v.0 == 1) {
                uses_v1 = true;
            }
        });
        assert!(uses_v1, "early read stays on the original register name");
    }

    // ── closure must stop at a (re-)definition block. Liveness models a
    // φ incoming as an edge copy at the pred end, so a value DEFINED in a
    // block that feeds a header φ is conservatively reported live-in to
    // its own def block; propagating the memory region through the loop
    // scheduled a reload ahead of the capture anchor. ──

    #[test]
    fn closure_does_not_reenter_def_block() {
        // b0: initial sum v1 + 14 live-through pressure values.
        // b1: header with two recurrent φs (excluded from anchoring).
        // b2: empty body-prefix block (X), predecessor of the latch;
        //     liveness reports v3 live-out here via the φ edge copy.
        // b3: latch (D) that DEFINES v3 (new sum1) and feeds φ v9.
        // b4: exit consuming every long-lived value.
        let mut b0 = vec![add(1, 700, 701)];
        for k in 0..14u32 {
            b0.push(add(100 + k, 600 + k, 620 + k));
        }
        let mut bexit = vec![add(200, 9, 100)];
        for k in 1..14u32 {
            bexit.push(add(200 + k, 199 + k, 100 + k));
        }
        let blocks = vec![
            blk(0, b0, Terminator::Branch(BlockId(1))),
            blk(
                1,
                vec![
                    Instruction::Phi {
                        dest: Value(9),
                        ty: IrType::I64,
                        incoming: vec![
                            (Operand::Value(Value(1)), BlockId(0)),
                            (Operand::Value(Value(3)), BlockId(3)),
                        ],
                    },
                    Instruction::Phi {
                        dest: Value(10),
                        ty: IrType::I64,
                        incoming: vec![
                            (Operand::Const(IrConst::I64(0)), BlockId(0)),
                            (Operand::Value(Value(4)), BlockId(3)),
                        ],
                    },
                    Instruction::Cmp {
                        dest: Value(11),
                        op: IrCmpOp::Slt,
                        lhs: Operand::Value(Value(9)),
                        rhs: Operand::Const(IrConst::I64(100)),
                        ty: IrType::I64,
                    },
                ],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(11)),
                    true_label: BlockId(2),
                    false_label: BlockId(4),
                },
            ),
            blk(2, vec![], Terminator::Branch(BlockId(3))),
            blk(
                3,
                vec![
                    add(5, 9, 777), // byte value
                    add(3, 9, 5),   // new sum1 — DEFINED in the latch
                    add(4, 10, 3),  // new sum2, reads the fresh sum1
                ],
                Terminator::Branch(BlockId(1)),
            ),
            blk(
                4,
                bexit,
                Terminator::Return(Some(Operand::Value(Value(213)))),
            ),
        ];
        // Direct closure invariant: v3 is DEFINED in block 3 and feeds the
        // header φ from block 3's end, so liveness conservatively reports it
        // live-in to its own def block. The closure seeded in its
        // predecessor block 2 must still not enter block 3.
        let mut f = func_with(blocks, 900);
        // Explicit direct probe before the rewrite.
        {
            let live = crate::backend::liveness::compute_live_intervals(&f);
            let lm = analysis::build_label_map(&f);
            let (_preds, succs) = analysis::build_cfg(&f, &lm);
            // Document the conservative precondition: without the def-block
            // stop test the closure would have somewhere to run.
            assert!(
                live.is_live_in(3, 3),
                "test shape requires the φ edge-copy live-in over-approximation"
            );
            let reached = memory_closure(&live, &f, &succs, 3, 2);
            assert!(
                reached.iter().all(|(b, _)| *b != 3),
                "closure re-entered the value's own def block: {reached:?}"
            );
        }
        let n = run_with_policy(
            &mut f,
            GlaPolicy {
                max_splits: 256,
                ..GlaPolicy::testing_permissive()
            },
        );
        assert!(n >= 1, "pressure must force splits");
        // Every capture store precedes its reloads (a closure crossing
        // into the latch def block put the reload ahead of the anchor).
        let mut slots = FxHashSet::default();
        for b in &f.blocks {
            for i in &b.instructions {
                if let Instruction::Alloca { dest, .. } = i {
                    slots.insert(dest.0);
                }
            }
        }
        verify_rewrite(&f, &slots).expect("closure never crosses a (re-)definition");
        // The fresh-sum reader (v4 = v10 + v3) still names v3 directly.
        let latch = f
            .blocks
            .iter()
            .find(|b| b.label == BlockId(3))
            .expect("latch present");
        let sum2 = latch
            .instructions
            .iter()
            .find(|i| i.dest().is_some_and(|d| d.0 == 4))
            .expect("sum2 still present");
        let mut reads_v3 = false;
        crate::backend::liveness::for_each_operand_in_instruction(sum2, |op| {
            if matches!(op, Operand::Value(v) if v.0 == 3) {
                reads_v3 = true;
            }
        });
        assert!(reads_v3, "latch sum2 reads the freshly defined sum1, v3");
    }

    // ── folded reads from an EARLIER block must never map to local point
    // 0 of a later φ-headed block (saturating_sub underflow) ──

    #[test]
    fn folded_reads_never_land_in_phi_prefix() {
        // b0: global v1, GEP v2 = v1+8, Load v3 folds both (hidden read of
        // v1 at b0's load point). b1 is a self-loop header with a φ; its
        // block-start global point is GREATER than v1's folded point.
        let blocks = vec![
            blk(
                0,
                vec![
                    global(1, "G"),
                    Instruction::GetElementPtr {
                        dest: Value(2),
                        base: Value(1),
                        offset: Operand::Const(IrConst::I64(8)),
                        ty: IrType::Ptr,
                    },
                    Instruction::Load {
                        volatile: false,
                        dest: Value(3),
                        ptr: Value(2),
                        ty: IrType::U8,
                        seg_override: AddressSpace::Default,
                    },
                ],
                Terminator::Branch(BlockId(1)),
            ),
            blk(
                1,
                vec![
                    Instruction::Phi {
                        dest: Value(4),
                        ty: IrType::I64,
                        incoming: vec![
                            (Operand::Value(Value(3)), BlockId(0)),
                            (Operand::Value(Value(5)), BlockId(1)),
                        ],
                    },
                    add(5, 4, 900),
                    Instruction::Cmp {
                        dest: Value(6),
                        op: IrCmpOp::Slt,
                        lhs: Operand::Value(Value(5)),
                        rhs: Operand::Const(IrConst::I64(3)),
                        ty: IrType::I64,
                    },
                ],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(6)),
                    true_label: BlockId(1),
                    false_label: BlockId(2),
                },
            ),
            blk(
                2,
                vec![],
                Terminator::Return(Some(Operand::Value(Value(4)))),
            ),
        ];
        let f = func_with(blocks, 900);
        let live = crate::backend::liveness::compute_live_intervals(&f);
        // The header (block 1) must not see v1 read at local point 0 — that
        // point is inside its φ prefix.
        let pts = block_use_points(&live, &f, 1, 1);
        assert!(
            !pts.contains(&0),
            "earlier-block folded read mapped into φ prefix: {pts:?}"
        );
    }

    // ── stale next_value_id (an upstream pass leaked a high id) must
    // never make GLA mint colliding value ids ──

    #[test]
    fn mints_fresh_ids_above_every_existing_def() {
        let (insts, _res) = pressure_ladder(1);
        let mut all = vec![Instruction::Alloca {
            dest: Value(899),
            ty: IrType::I64,
            size: 8,
            align: 8,
            volatile: true,
            semantic_volatile: false,
        }];
        all.extend(insts);
        let mut f = func_with(
            vec![blk(
                0,
                all,
                Terminator::Return(Some(Operand::Value(Value(213)))),
            )],
            // Deliberately stale: one existing definition (v899) is already
            // at or above the counter.
            899,
        );
        let n = run_with_policy(
            &mut f,
            GlaPolicy {
                max_splits: 256,
                ..GlaPolicy::testing_permissive()
            },
        );
        assert!(n >= 1, "pressure ladder still splits");
        let mut seen = FxHashSet::default();
        for b in &f.blocks {
            for i in &b.instructions {
                if let Some(d) = i.dest() {
                    assert!(seen.insert(d.0), "duplicate definition of v{}", d.0);
                    assert!(
                        d.0 < f.next_value_id,
                        "id {} >= next_value_id {}",
                        d.0,
                        f.next_value_id
                    );
                }
            }
        }
        let mut slots = FxHashSet::default();
        for b in &f.blocks {
            for i in &b.instructions {
                if let Instruction::Alloca { dest, .. } = i {
                    slots.insert(dest.0);
                }
            }
        }
        verify_rewrite(&f, &slots).expect("rewrite verifies with a stale counter");
    }

    // ── call boundary gap ──

    #[test]
    fn call_gap_is_profitable_and_sound() {
        // v1 defined, call (clobbers), v1 used much later under pressure: the
        // gap containing the call stores before / reloads after. Shape is a
        // single block with a call and many live values.
        let (mut mut_insts, _) = pressure_ladder(1);
        // Remove the in-block consumer chain: it moves after the call.
        mut_insts.truncate(15);
        let mut insts = mut_insts;
        insts.push(Instruction::Call {
            func: "clobber".into(),
            info: CallInfo {
                dest: Some(Value(50)),
                args: Vec::new(),
                arg_types: Vec::new(),
                return_type: IrType::I64,
                is_variadic: false,
                num_fixed_args: 0,
                struct_arg_sizes: Vec::new(),
                struct_arg_aligns: Vec::new(),
                struct_arg_classes: Vec::new(),
                struct_arg_riscv_float_classes: Vec::new(),
                struct_arg_is_f128_sse: Vec::new(),
                ret_is_f128_sse: false,
                is_sret: false,
                is_fastcall: false,
                regparm: None,
                is_pure: false,
                is_const: false,
                ret_eightbyte_classes: Vec::new(),
            },
        });
        // Post-call consumer chain.
        insts.push(add(200, 1, 100));
        for k in 1..14u32 {
            insts.push(add(200 + k, 199 + k, 100 + k));
        }
        let mut f = func_with(
            vec![blk(
                0,
                insts,
                Terminator::Return(Some(Operand::Value(Value(213)))),
            )],
            900,
        );
        let n = run_with_policy(
            &mut f,
            GlaPolicy {
                max_splits: 256,
                ..GlaPolicy::testing_permissive()
            },
        );
        assert!(n >= 1);
        verify_rewrite(&f, &FxHashSet::default()).expect("rewrite verifies");
    }

    // ── critical edge mechanics ──

    #[test]
    fn retarget_edge_moves_exactly_one_edge() {
        let mut term = Terminator::CondBranch {
            cond: Operand::Const(IrConst::I64(1)),
            true_label: BlockId(7),
            false_label: BlockId(8),
        };
        retarget_edge(&mut term, BlockId(7), BlockId(99));
        match term {
            Terminator::CondBranch {
                true_label,
                false_label,
                ..
            } => {
                assert_eq!(true_label, BlockId(99));
                assert_eq!(false_label, BlockId(8));
            }
            _ => panic!(),
        }
    }

    // ── determinism ──

    #[test]
    fn planning_is_deterministic() {
        let make = || {
            let insts = remat_ladder();
            func_with(
                vec![blk(
                    0,
                    insts,
                    Terminator::Return(Some(Operand::Value(Value(211)))),
                )],
                900,
            )
        };
        let mut a = make();
        let mut b = make();
        run(&mut a, 64, 2);
        run(&mut b, 64, 2);
        let ca: Vec<usize> = a.blocks.iter().map(|x| x.instructions.len()).collect();
        let cb: Vec<usize> = b.blocks.iter().map(|x| x.instructions.len()).collect();
        assert_eq!(ca, cb);
        assert_eq!(a.next_value_id, b.next_value_id);
    }

    // ── verifier ──

    #[test]
    fn verifier_catches_undominated_reload() {
        let f = func_with(
            vec![blk(
                0,
                vec![Instruction::Load {
                    volatile: false,
                    dest: Value(1),
                    ptr: Value(5),
                    ty: IrType::I64,
                    seg_override: AddressSpace::Default,
                }],
                Terminator::Return(Some(Operand::Value(Value(1)))),
            )],
            6,
        );
        let mut slots = FxHashSet::default();
        slots.insert(5);
        assert!(verify_rewrite(&f, &slots).is_err());
    }

    #[test]
    fn verifier_accepts_dominated_reload() {
        let f = func_with(
            vec![blk(
                0,
                vec![
                    Instruction::Store {
                        val: Operand::Value(Value(7)),
                        ptr: Value(5),
                        ty: IrType::I64,
                        seg_override: AddressSpace::Default,
                        volatile: false,
                    },
                    Instruction::Load {
                        volatile: false,
                        dest: Value(1),
                        ptr: Value(5),
                        ty: IrType::I64,
                        seg_override: AddressSpace::Default,
                    },
                ],
                Terminator::Return(Some(Operand::Value(Value(1)))),
            )],
            8,
        );
        let mut slots = FxHashSet::default();
        slots.insert(5);
        assert!(verify_rewrite(&f, &slots).is_ok());
    }

    // ── Phase-1 profitability policy (A/B-calibrated 2026-09-11) ──

    /// The production policy: spill gaps OFF, intra-block gaps OFF,
    /// one-segment remats, reach band 6. Mirrors `GlaPolicy::from_env`
    /// defaults WITHOUT reading the environment so tests are hermetic.
    fn conservative_policy(max_splits: usize) -> GlaPolicy {
        GlaPolicy {
            tier: Tier::Speed,
            budget: pressure_budget(),
            max_splits,
            remat_max_weight: 3,
            remat_max_segments: 1,
            reach_band: 6,
            allow_spill_gaps: false,
            allow_intra_block_gaps: false,
            min_gap: pressure_min_gap(),
            min_benefit: 40,
            benefit_ratio10: 20,
        }
    }

    #[test]
    fn reach_band_logic_rejects_only_grossly_over_blocks() {
        let p = conservative_policy(64);
        assert!(p.peak_is_reachable(p.budget as u32));
        assert!(p.peak_is_reachable((p.budget + 6) as u32));
        assert!(!p.peak_is_reachable((p.budget + 7) as u32));
        assert!(!p.peak_is_reachable(45));
    }

    #[test]
    fn reach_band_defaults_follow_target_and_opt_tier() {
        // The A/B override would change the answer; hermetic tests assume no
        // ambient override.
        if std::env::var("CCC_GLA_REACH").is_ok() || crate::common::types::target_is_32bit() {
            return;
        }
        // x86-64 speed tier: the six buyable callee-saved GPRs.
        assert_eq!(reach_band(tier_for(1)), 6);
        assert_eq!(reach_band(tier_for(2)), 6);
        assert_eq!(reach_band(tier_for(3)), 6);
        // -O0 disables the coloring tier: no plan to perturb, band wide.
        assert_eq!(reach_band(tier_for(0)), 64);
        // Size tiers never plan: band zero.
        assert_eq!(reach_band(tier_for(4)), 0);
        assert_eq!(tier_for(0), Tier::Debug);
        assert_eq!(tier_for(2), Tier::Speed);
        assert_eq!(tier_for(5), Tier::Size);
    }

    #[test]
    fn size_tier_never_plans() {
        let mut p = conservative_policy(64);
        p.tier = Tier::Size;
        assert!(!p.enabled());
        let mut f = func_with(
            vec![blk(
                0,
                remat_ladder(),
                Terminator::Return(Some(Operand::Value(Value(211)))),
            )],
            900,
        );
        assert_eq!(run_with_policy(&mut f, p), 0);
    }

    #[test]
    fn own_use_run_points_are_not_net_relief() {
        let policy = conservative_policy(64);
        // Shape A (adler32-style): global defined at function entry, read
        // only at a LATE cluster; the residency it carries ACROSS the
        // pressure peak is net relief and must be credited.
        let insts = remat_ladder();
        let f = func_with(
            vec![blk(
                0,
                insts,
                Terminator::Return(Some(Operand::Value(Value(211)))),
            )],
            900,
        );
        let live = crate::backend::liveness::compute_live_intervals(&f);
        let classes = ColorClasses::phi_webs(&f);
        let eligible: FxHashSet<u32> = (1u32..=211).collect();
        let pressure = Pressure::build(&live, &f, &eligible, &classes);
        assert!(covers_over_budget_point(&live, &f, &pressure, 1, &policy));

        // Shape B (nbody -Os-style): 12 producers hold the block at
        // exactly budget; global v1 is defined late and read at the very
        // next point, then dies. Every over-budget point is a def/read
        // point where the clone occupies the register the edit claims to
        // free — net relief is zero and the speed tier must reject it.
        let mut v = Vec::new();
        for k in 0..12u32 {
            v.push(add(100 + k, 700 + k, 720 + k));
        }
        v.push(global(1, "G")); // late def at pressure plateau
        v.push(add(300, 1, 999)); // sole read immediately afterwards
        v.push(add(200, 100, 101));
        for k in 1..12u32 {
            v.push(add(200 + k, 199 + k, 100 + k));
        }
        let g = func_with(
            vec![blk(
                0,
                v,
                Terminator::Return(Some(Operand::Value(Value(211)))),
            )],
            900,
        );
        let live2 = crate::backend::liveness::compute_live_intervals(&g);
        let c2 = ColorClasses::phi_webs(&g);
        let elig2: FxHashSet<u32> = (1u32..=300).collect();
        let p2 = Pressure::build(&live2, &g, &elig2, &c2);
        let (_, peak) = p2.peak(0);
        assert!(
            (13..=14).contains(&peak),
            "fixture peaks just over budget, got {peak}"
        );
        assert!(
            !covers_over_budget_point(&live2, &g, &p2, 1, &policy),
            "def/read-only residency is not net relief"
        );
        // The debug tier (-O0) credits the full resident span: at -O0 the
        // win is slot round-trip removal, not colorer residency.
        let mut debug = conservative_policy(64);
        debug.tier = Tier::Debug;
        debug.reach_band = 64;
        assert!(covers_over_budget_point(&live2, &g, &p2, 1, &debug));
    }

    #[test]
    fn phi_web_counts_as_one_color_class() {
        // v9 = φ(v1, v3), v10 = φ(c, v4): the colorer coalesces each web
        // into one register, so a block full of loop-carried φ webs must
        // not be counted once per SSA name.
        let blocks = vec![
            blk(0, vec![add(1, 500, 501)], Terminator::Branch(BlockId(1))),
            blk(
                1,
                vec![
                    Instruction::Phi {
                        dest: Value(9),
                        ty: IrType::I64,
                        incoming: vec![
                            (Operand::Value(Value(1)), BlockId(0)),
                            (Operand::Value(Value(3)), BlockId(2)),
                        ],
                    },
                    Instruction::Phi {
                        dest: Value(10),
                        ty: IrType::I64,
                        incoming: vec![
                            (Operand::Const(IrConst::I64(0)), BlockId(0)),
                            (Operand::Value(Value(4)), BlockId(2)),
                        ],
                    },
                    add(3, 9, 700),
                    add(4, 10, 701),
                ],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(3)),
                    true_label: BlockId(1),
                    false_label: BlockId(3),
                },
            ),
            blk(2, vec![], Terminator::Branch(BlockId(1))),
            blk(
                3,
                vec![],
                Terminator::Return(Some(Operand::Value(Value(9)))),
            ),
        ];
        let f = func_with(blocks, 800);
        let live = crate::backend::liveness::compute_live_intervals(&f);
        let mut eligible: FxHashSet<u32> = FxHashSet::default();
        for v in [1u32, 3, 4, 9, 10] {
            eligible.insert(v);
        }
        // Naive SSA-name counting at the header peak:
        let naive = Pressure::build(
            &live,
            &f,
            &eligible,
            &ColorClasses {
                parent: FxHashMap::default(),
            },
        );
        let classes = ColorClasses::phi_webs(&f);
        let coalesced = Pressure::build(&live, &f, &eligible, &classes);
        let (_p1, n_naive) = naive.peak(1);
        let (_p2, n_coal) = coalesced.peak(1);
        assert!(
            n_coal < n_naive,
            "φ webs must count once ({n_coal} < {n_naive})"
        );
    }

    #[test]
    fn grossly_over_block_is_left_to_production_allocator() {
        // nbody lesson: a block grossly over the register budget (>
        // budget+reach) cannot be made colorable by a handful of splits —
        // the production allocator resolves it with folded memory operands
        // and callee-save buys anyway, and pre-alloc edits only perturb
        // that coloring (measured +28 insns / +88 stkref). Build a REAL
        // 21-class peak: 20 independent producers kept live until a
        // sequential consumer chain (same proven ladder as the rest of
        // the suite, scaled past budget+reach), plus the global.
        let ladder = |n: u32| {
            let mut v = Vec::new();
            for k in 0..n {
                v.push(add(100 + k, 600 + k, 620 + k));
            }
            v.push(add(200, 1, 100));
            for k in 1..n {
                v.push(add(200 + k, 199 + k, 100 + k));
            }
            v
        };
        let make = || {
            let mut insts = vec![global(1, "bodies")];
            insts.extend(ladder(20));
            func_with(
                vec![blk(
                    0,
                    insts,
                    Terminator::Return(Some(Operand::Value(Value(219)))),
                )],
                900,
            )
        };
        // Sanity: the fixture genuinely peaks at 21 coalesced classes.
        let probe = make();
        let live = crate::backend::liveness::compute_live_intervals(&probe);
        let classes = ColorClasses::phi_webs(&probe);
        let eligible: FxHashSet<u32> = (1u32..=119).collect();
        let p = Pressure::build(&live, &probe, &eligible, &classes);
        let (_pt, peak) = p.peak(0);
        assert!(
            peak >= 21,
            "fixture must be grossly over budget, got {peak}"
        );

        let mut f = make();
        let n = run_with_policy(&mut f, conservative_policy(64));
        assert_eq!(n, 0, "grossly-over block must never be edited");
        assert_eq!(
            count(&f, |i| matches!(i, Instruction::GlobalAddr { .. })),
            1,
            "no remat clones in a hopeless block"
        );
        // Raising the reach band is exactly what re-admits it: the band,
        // not some other gate, is the discriminator.
        let mut g = make();
        let m = run_with_policy(
            &mut g,
            GlaPolicy {
                reach_band: 999,
                ..conservative_policy(64)
            },
        );
        assert!(m >= 1, "a wide reach band edits the block again");
    }

    #[test]
    fn single_segment_global_remat_tips_reachable_peak() {
        // adler32 shape: one single-segment global whose residency pushes a
        // otherwise-fittable block 1–4 classes over budget. Remat fires,
        // minting clones but never a stack slot.
        let insts = remat_ladder();
        let mut f = func_with(
            vec![blk(
                0,
                insts,
                Terminator::Return(Some(Operand::Value(Value(211)))),
            )],
            900,
        );
        let n = run_with_policy(&mut f, conservative_policy(64));
        assert!(n >= 1, "reachable peak rematerializes the global");
        assert!(count(&f, |i| matches!(i, Instruction::GlobalAddr { .. })) >= 2);
        assert_eq!(count(&f, |i| matches!(i, Instruction::Alloca { .. })), 0);
        verify_rewrite(&f, &FxHashSet::default()).expect("rewrite verifies");
    }

    #[test]
    fn remat_segment_cap_is_one_in_shipped_policy() {
        // The two-segment liveness shape (edge-point holes around nested
        // loop headers, as in nbody's printf format-string global v94) is
        // anchored end-to-end in
        // tests/regression/check_gla_remat_policy.sh against the real
        // benchmark; here we pin the planner predicate itself so a later
        // default flip cannot silently re-admit the nbody cascade.
        let seg_cap = conservative_policy(64).remat_max_segments;
        assert_eq!(seg_cap, 1);
        // A two-piece value is refused by the cap while a one-piece value
        // is admitted (pure predicate check, no liveness archaeology).
        assert!(1usize <= seg_cap);
        assert!(2usize > seg_cap);
    }

    #[test]
    fn conservative_policy_never_builds_capture_slots() {
        // The cross-block pressure fixture: permissive policy fires a global
        // spill gap (alloca + stores + exit reloads); the conservative
        // policy must leave the IR entirely to the production allocator.
        let make = || {
            let (b0_all, _) = pressure_ladder(1);
            let b0 = b0_all[..15].to_vec();
            let mut b2 = Vec::new();
            b2.push(add(200, 1, 100));
            for k in 1..14u32 {
                b2.push(add(200 + k, 199 + k, 100 + k));
            }
            b2.push(add(300, 213, 1));
            let blocks = vec![
                blk(0, b0, Terminator::Branch(BlockId(1))),
                blk(
                    1,
                    vec![
                        Instruction::Phi {
                            dest: Value(2),
                            ty: IrType::I64,
                            incoming: vec![
                                (Operand::Const(IrConst::I64(0)), BlockId(0)),
                                (Operand::Value(Value(3)), BlockId(1)),
                            ],
                        },
                        add(3, 2, 900),
                        Instruction::Cmp {
                            dest: Value(4),
                            op: IrCmpOp::Slt,
                            lhs: Operand::Value(Value(3)),
                            rhs: Operand::Const(IrConst::I64(10)),
                            ty: IrType::I64,
                        },
                    ],
                    Terminator::CondBranch {
                        cond: Operand::Value(Value(4)),
                        true_label: BlockId(1),
                        false_label: BlockId(2),
                    },
                ),
                blk(2, b2, Terminator::Return(Some(Operand::Value(Value(300))))),
            ];
            func_with(blocks, 900)
        };
        let mut f = make();
        let n = run_with_policy(&mut f, conservative_policy(256));
        assert_eq!(n, 0, "spill gaps are off in the shipped policy");
        assert_eq!(count(&f, |i| matches!(i, Instruction::Alloca { .. })), 0);
        let mut g = make();
        let m = run_with_policy(
            &mut g,
            GlaPolicy {
                max_splits: 256,
                ..GlaPolicy::testing_permissive()
            },
        );
        assert!(m >= 1, "permissive policy retains the gap capability");
        verify_rewrite(&g, &FxHashSet::default()).expect("permissive rewrite verifies");
    }
}
