//! GLA policy knobs and tier model — split from the former monolithic `location_alloc.rs`.
//! See the module docs in [`super`] for the full design and policy record.

use super::*;

// ─────────────────────────────────────────────────────────────────────────────
// Tunables (env-overridable for A/B work; clamped)
// ─────────────────────────────────────────────────────────────────────────────

/// Master gate for the global location allocator.
///
/// **Default: ON.** The source-less rematerialization policy this gate
/// controls shipped after a full-corpus census (785 TUs × {-O0..-O3,-Os} ×
/// {x86-64,i686}): every applied edit was a source-less remat (GlobalAddr or
/// Copy-of-const) with zero capture slots, zero failed rewrites and gate
/// on/off runtime equivalence; static deltas are net-negative on both
/// targets and hot-loop Callgrind is at worst neutral (-1.45% Ir on
/// zlib_ng_adler32, geomean 0.99953, see engineering/DECISIONS.md
/// RA-GLA-01).
///
/// The gate remains available as an emergency kill switch: setting
/// `CCC_RA_GLOBAL_LOCATION` to `0`, `off`, `no`, `false` (any case) or the
/// empty string disables the pass. Any other value (incl. `1`/`on`)
/// enables it; an unset variable follows the default (enabled).
/// Whether `value` is one of the documented OFF tokens (case-insensitive):
/// `0`, `off`, `no`, `false` or the empty string. Every GLA boolean knob
/// parses through this one helper so the emergency-kill-switch vocabulary
/// cannot drift between knobs.
fn is_off_token(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().as_str(),
        "0" | "off" | "no" | "false" | ""
    )
}

pub(crate) fn gate_enabled() -> bool {
    // Unset follows the default (enabled); only an explicit off token
    // disables the pass.
    match std::env::var("CCC_RA_GLOBAL_LOCATION").as_deref() {
        Err(_) => true,
        Ok(v) => !is_off_token(v),
    }
}

/// Parse an opt-in boolean knob: unset and every off token ⇒ `default`;
/// any other value enables it. Used for the shipped-OFF experimental
/// switches so an emergency `=off`/`=no`/empty works exactly like the
/// master gate instead of silently enabling the feature.
fn opt_in_env(name: &str, default: bool) -> bool {
    match std::env::var(name) {
        Err(_) => default,
        Ok(v) if is_off_token(&v) => false,
        Ok(_) => true,
    }
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
pub(super) fn remat_max_weight() -> u64 {
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
pub(super) fn benefit_ratio10() -> u64 {
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

/// Whether explicit spill gaps (capture store + reload pieces) may be
/// planned at all. The Phase-1 A/B census (`scripts/ra_quality_census.py`,
/// 78-file corpus) measured pre-alloc gap insertion as net-negative in
/// every configuration: the pressure proxy counts points the production
/// colorer resolves for free by folding memory operands, and the gaps that
/// do fire mainly replace once-per-call callee-save pushes with per-
/// iteration reload traffic. Gap support stays in the code (it is the
/// substrate P0-B cyclic-φ and post-alloc feedback builds on) but defaults
/// OFF; the whole feature is additionally behind `CCC_RA_GLOBAL_LOCATION`.
pub(super) fn allow_spill_gaps() -> bool {
    static S: OnceLock<bool> = OnceLock::new();
    *S.get_or_init(|| opt_in_env("CCC_GLA_SPILL_GAPS", false))
}

/// Whether same-block reload gaps are allowed. The RA-06 decision record
/// measured intra-block pre-alloc spill-then-reload as net-negative on every
/// corpus program: the production allocator's own evictions fold memory
/// operands into the consuming instruction at zero extra instructions, while
/// an explicit gap always pays an uncoalescable store + reload. The global
/// allocator's unique value is CROSS-block (and cross-call) pressure, so
/// same-block gaps are rejected unless explicitly re-enabled for A/B work.
pub(super) fn allow_intra_block_gaps() -> bool {
    static A: OnceLock<bool> = OnceLock::new();
    *A.get_or_init(|| opt_in_env("CCC_GLA_ALLOW_INTRA", false))
}

/// Which production allocator tier the planner prepares code for. See
/// [`Tier`] for what differs between the three operating points.
pub(super) fn tier_for(opt_level: u32) -> Tier {
    match opt_level {
        0 => Tier::Debug,
        4 | 5 => Tier::Size,
        _ => Tier::Speed,
    }
}

/// Callee-saved GPRs the production allocator can additionally BUY with
/// prologue/epilogue push/pop traffic on x86-64 SysV: %rbx, %rbp and
/// %r12–%r15 — exactly six homes beyond the caller-saved scan budget.
/// The x86-64 reach band's Speed value is derived from this set so the
/// accounting cannot silently drift from the ABI facts.
pub(super) const X86_64_BUYABLE_CALLEE_SAVED_GPRS: &[&str] =
    &["rbx", "rbp", "r12", "r13", "r14", "r15"];

/// Callee-saved GPRs freely buyable on i686 SysV **under PIC**: the ABI
/// also preserves %ebx and %ebp, but under PIC %ebx is the GOT base and
/// %ebp is the frame pointer, leaving only %esi/%edi. A non-PIC compile
/// could in principle add %ebx, but the 3-wide band was never validated
/// (it already re-admits the loop_patterns counter spill, 1.038× runtime;
/// 6-wide is 1.053×) — the calibrated value stays fail-closed at two.
pub(super) const I686_BUYABLE_CALLEE_SAVED_GPRS: &[&str] = &["esi", "edi"];

/// Residual GPR residency the production allocator can bring to bear
/// beyond [`pressure_budget`] by buying callee-saved homes — the physical
/// quantity the Speed reach band proxies (see the band calibration in
/// [`reach_band`]). aarch64 and riscv64 deliberately inherit the
/// x86-64-derived band and the conservative GPR scan budget (12 vs ~27
/// allocatable GPRs on either ISA): the model then fires only at pressure
/// levels that are genuinely extreme for those register files, i.e. it
/// can only UNDER-fire, never over-fire. Both targets carry full
/// gate-on/gate-off/gcc equivalence matrices under qemu-user plus a
/// trampoline trace proof in RA-GLA-02; a target-specific calibration is
/// a future optimization, not a safety prerequisite.
pub(super) fn buyable_callee_saved_gprs() -> usize {
    if crate::common::types::target_is_32bit() {
        I686_BUYABLE_CALLEE_SAVED_GPRS.len()
    } else {
        X86_64_BUYABLE_CALLEE_SAVED_GPRS.len()
    }
}

/// Relief band: a block whose peak exceeds the register budget by more than
/// this many color classes is not made colorable by a handful of splits —
/// the production allocator will stack-allocate there regardless, so any
/// edits whose only covered over-budget points lie in such blocks add
/// capture traffic for zero register residency, and (worse) can reshuffle
/// the colorer's choices in the blocks it *would* have handled optimally.
///
/// The number proxies the residual register capacity the production
/// allocator can still bring to bear beyond the scan budget, i.e.
/// [`buyable_callee_saved_gprs`]:
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
pub(super) fn reach_band(tier: Tier) -> u32 {
    if let Ok(v) = std::env::var("CCC_GLA_REACH") {
        if let Ok(n) = v.parse::<u32>() {
            return n.clamp(0, 64);
        }
    }
    match tier {
        // No coloring tier to perturb: any pressure relief is direct.
        Tier::Debug => 64,
        // Residual callee-save capacity beyond the scan budget; the number
        // is derived from the named register sets, not a magic literal.
        Tier::Speed => buyable_callee_saved_gprs() as u32,
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
pub(super) fn remat_max_segments() -> usize {
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
pub(super) fn min_benefit() -> u64 {
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
pub(super) enum Tier {
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
pub(super) struct GlaPolicy {
    /// Production allocator tier this plan is prepared for.
    pub(super) tier: Tier,
    /// Register residency budget (GPR color classes per point).
    pub(super) budget: usize,
    /// Total edit budget (remats + gaps) per function.
    pub(super) max_splits: usize,
    /// Dynamic-use weight above which a value keeps a register instead of
    /// being rematerialized.
    pub(super) remat_max_weight: u64,
    /// Maximum hole-aware live segments a rematted value may have.
    pub(super) remat_max_segments: usize,
    /// Excess classes over budget still considered salvageable.
    pub(super) reach_band: u32,
    /// Whether capture-store/reload gaps may be planned at all.
    pub(super) allow_spill_gaps: bool,
    /// Whether gaps whose both register pieces are in the same block count.
    pub(super) allow_intra_block_gaps: bool,
    /// Minimum point span for an in-block gap.
    pub(super) min_gap: usize,
    /// Minimum weighted benefit a gap must promise.
    pub(super) min_benefit: u64,
    /// Required benefit/cost ratio in tenths (20 = 2.0×).
    pub(super) benefit_ratio10: u64,
}

impl GlaPolicy {
    pub(super) fn from_env(max_splits: usize, opt_level: u32) -> Self {
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
    pub(super) fn peak_is_reachable(&self, pk: u32) -> bool {
        pk as usize <= self.budget + self.reach_band as usize
    }

    /// Size tiers never plan: see [`Tier::Size`].
    pub(super) fn enabled(&self) -> bool {
        self.tier != Tier::Size
    }

    /// Whether residency relief must be net of the value's own def/read
    /// spans. True for the coloring allocator tier (a clone re-occupies a
    /// register at every use run); false at -O0 where every value is
    /// stack-homed and even use-point remats remove slot round-trips.
    pub(super) fn net_relief(&self) -> bool {
        self.tier == Tier::Speed
    }
}

#[cfg(test)]
impl GlaPolicy {
    /// Fully permissive policy for mechanism tests (gaps ON, including
    /// intra-block; generous reach/ratio/segment limits).
    pub(super) fn testing_permissive() -> Self {
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

#[cfg(test)]
mod token_tests {
    use super::{is_off_token, opt_in_env};

    #[test]
    fn off_tokens_are_case_insensitive_and_exhaustive() {
        for tok in [
            "0", "off", "OFF", "Off", "no", "NO", "false", "FALSE", "False", "",
        ] {
            assert!(is_off_token(tok), "expected {tok:?} to be an off token");
        }
    }

    #[test]
    fn on_tokens_are_everything_else() {
        for tok in ["1", "on", "ON", "true", "yes", "2", "enable"] {
            assert!(!is_off_token(tok), "expected {tok:?} to enable");
        }
    }

    #[test]
    fn opt_in_env_defaults_off_for_unset_and_honors_both_vocabularies() {
        // Unique throwaway vars: no OnceLock caches these call sites, so
        // mutation cannot race other tests (edition 2024 marks env
        // mutation unsafe for that reason).
        // SAFETY: test-only, uniquely named variables, no concurrent reader.
        unsafe {
            const NAME: &str = "CCC_GLA_TEST_OPT_IN_TOKEN_PARSER";
            std::env::remove_var(NAME);
            assert!(!opt_in_env(NAME, false));
            assert!(opt_in_env("CCC_GLA_TEST_OPT_IN_TOKEN_PARSER_ON", true));
            std::env::set_var(NAME, "1");
            assert!(opt_in_env(NAME, false));
            std::env::set_var(NAME, "No");
            assert!(!opt_in_env(NAME, true));
            std::env::set_var(NAME, "");
            assert!(!opt_in_env(NAME, true));
            std::env::remove_var(NAME);
        }
    }
}
