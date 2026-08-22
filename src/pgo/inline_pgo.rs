//! Profile-guided inlining with a bounded, frequency-aware budget.
//!
//! Hotness classification is DATA-DRIVEN via the profile summary
//! (`summary::ProfileSummary`, an LLVM `ProfileSummaryInfo` analogue):
//! percentile thresholds computed from the unit's count distribution replace
//! the fixed magic ratios (`relative_frequency >= 0.10/0.05/0.005`).
//! Per-call-site hotness uses the derived count of the block containing the
//! call (LLVM `isHotCallSite`/BFI), which is a strictly stronger signal than
//! function entry counts for call sites in hot loops.
use crate::ir::reexports::IrFunction;
use crate::pgo::ProfileData;

fn size_opt() -> bool {
    std::env::var("CFLAGS")
        .map(|s| s.contains("-Os") || s.contains("-Oz"))
        .unwrap_or(false)
}

fn entry_counts(caller: &IrFunction, callee: &IrFunction, p: &ProfileData) -> (u64, u64) {
    match crate::pgo::active_unit() {
        Some(u) => (
            crate::pgo::summary::entry_count_for(p, &u, &caller.name),
            crate::pgo::summary::entry_count_for(p, &u, &callee.name),
        ),
        None => (
            p.get(&caller.name).map(|f| f.total_count).unwrap_or(0),
            p.get(&callee.name).map(|f| f.total_count).unwrap_or(0),
        ),
    }
}

/// Summary-driven threshold multiplier. When no summary is available the
/// legacy relative-frequency ratios are used as a fallback.
pub fn inline_threshold_multiplier(caller: &str, callee: &str, p: &ProfileData) -> f64 {
    let cf = p.relative_frequency(caller);
    let df = p.relative_frequency(callee);
    if let Some(s) = crate::pgo::summary::get_summary() {
        let to_count = |r: f64| (r * s.max as f64) as u64;
        let ch = s.is_hot(to_count(cf));
        let dh = s.is_hot(to_count(df));
        let dc = s.is_cold(to_count(df));
        let cc = s.is_cold(to_count(cf));
        // The hot-caller/hot-callee bonus is only meaningful when the
        // profile has genuine hot/cold separation. On a flat profile
        // (`has_spread()` == false) "both hot" is vacuous, and returning 1.75
        // makes the base inliner over-inline helpers into hot functions —
        // restructuring them and regressing the hot path (see summary.rs).
        if ch && dh {
            if s.has_spread() {
                1.75
            } else {
                1.0
            }
        } else if ch && dc {
            0.60
        } else if cc {
            0.80
        } else {
            1.0
        }
    } else {
        // legacy relative-frequency fallback (no summary): same rule — the 1.75
        // bonus needs a real hot/cold spread, else it over-inlines.
        let m = p
            .functions
            .values()
            .map(|x| x.total_count)
            .max()
            .unwrap_or(0);
        // A genuine hot/cold spread exists if some function is at least 10x
        // rarer than the hottest — hotness classification is then meaningful.
        let has_cold = m > 0
            && p.functions
                .values()
                .any(|f| f.total_count.saturating_mul(10) < m.max(1));
        if cf >= 0.10 && df >= 0.05 && has_cold {
            1.75
        } else if cf >= 0.10 && df < 0.005 {
            0.60
        } else if cf < 0.005 {
            0.80
        } else {
            1.0
        }
    }
}

/// Whether profile-driven inlining decisions are meaningfully active.
///
/// On a FLAT profile (no hot/cold spread) every call site and function looks
/// "hot" to the percentile thresholds, so the PGO inliner has no informative
/// signal. Merely reading the profile in that state can perturb the inliner's
/// pass iterations and change the final code, regressing hot paths (measured
/// finding: zlib-ng adler32 and expat kernels regressed ~20-26% under
/// `-fprofile-use` on flat profiles, and the regression survived disabling
/// layout and the identifiable consumers). Gate the entire PGO inline filter on
/// this: a flat profile compiles exactly like plain `-O2` for inlining, and PGO
/// inlining only engages where the profile is genuinely skewed.
pub fn inline_decisions_active() -> bool {
    if std::env::var("LCCC_PGO_NO_INLINE").is_ok() {
        return false;
    }
    match crate::pgo::summary::get_summary() {
        Some(s) => s.has_spread(),
        None => true, // no summary: use the relative-frequency heuristics
    }
}

/// Entry-count-only decision (call sites without block info).
pub fn should_inline_normal(
    caller: &IrFunction,
    callee: &IrFunction,
    p: Option<&ProfileData>,
) -> Option<bool> {
    let p = p?;
    let (cf, df) = entry_counts(caller, callee, p);
    should_inline_impl(caller, callee, cf, df, 0, p)
}

/// Call-site-aware decision. `site_count` is the derived count of the
/// block containing the call (0 when unknown → entry-count-only behavior).
pub fn should_inline_site(
    caller: &IrFunction,
    callee: &IrFunction,
    site_count: u64,
    p: Option<&ProfileData>,
) -> Option<bool> {
    let p = p?;
    let (cf, df) = entry_counts(caller, callee, p);
    should_inline_impl(caller, callee, cf, df, site_count, p)
}

fn should_inline_impl(
    caller: &IrFunction,
    callee: &IrFunction,
    cf: u64,
    df: u64,
    site_count: u64,
    p: &ProfileData,
) -> Option<bool> {
    let size: usize = callee.blocks.iter().map(|b| b.instructions.len()).sum();
    let (caller_hot, callee_hot, caller_cold, callee_cold, site_hot, site_cold) =
        match crate::pgo::summary::get_summary() {
            Some(s) => (
                s.is_hot(cf),
                df > 0 && s.is_hot(df),
                cf > 0 && s.is_cold(cf),
                df > 0 && s.is_cold(df),
                site_count > 0 && s.is_hot(site_count),
                site_count > 0 && s.is_cold(site_count),
            ),
            None => {
                // Fallback: relative-frequency ratios.
                let m = p
                    .functions
                    .values()
                    .map(|x| x.total_count)
                    .max()
                    .unwrap_or(0);
                let r = |c: u64| if m == 0 { 0.0 } else { c as f64 / m as f64 };
                (
                    r(cf) >= 0.10,
                    df > 0 && r(df) >= 0.05,
                    cf > 0 && r(cf) < 0.005,
                    df > 0 && r(df) < 0.001,
                    false,
                    false,
                )
            }
        };
    if size_opt() {
        if (caller_hot || site_hot) && callee_hot && size <= 12 {
            return Some(true);
        }
        if (callee_cold || site_cold) && size > 12 {
            return Some(false);
        }
        return None;
    }
    // Strongest signal: a HOT CALL SITE (the block containing the call is
    // hot — LLVM HotCallSiteThreshold). Medium hot callees inline. This is
    // only reached when inline_decisions_active() (a non-flat profile), so the
    // percentile hotness is informative.
    if (caller_hot || site_hot) && callee_hot && size <= 48 {
        return Some(true);
    }
    // A LOOPED hot call site — the site executes substantially more often
    // than its caller's entry (site_count >> caller entry), i.e. it is inside a
    // hot loop. Inlining it removes call overhead from every iteration and
    // exposes the callee to the caller's optimizations, which a plain build
    // cannot do when the base inliner skips the (larger) callee. This is the
    // PGO advantage LLVM/ICC rely on; allow larger callees here, still bounded.
    if site_count > 0 && site_count.saturating_mul(2) >= cf.max(1) && size <= 200 {
        return Some(true);
    }
    // ENTRY-COUNT-RATIO force-inline. Per-block site counts are
    // unavailable at pre-pass time — the pre-pass CFG's block labels differ
    // from the post-pass labels the instrumentation recorded, so
    // `derive_block_counts` yields zero and the block-level hot-site signal is
    // destroyed (this is exactly why a block-count force-inline never fires). The
    // function ENTRY counts, however, are LABEL-INDEPENDENT and survive across
    // pre/post pass. The ratio `df / cf` is the average number of times the
    // callee runs per caller invocation: when it is large, the callee is called
    // from a loop inside the caller, i.e. a hot call site — even if the base
    // inliner would skip the (larger) callee. Use it as a robust, block-free
    // proxy for looped-hotness. Only on a non-flat (informative) profile.
    if crate::pgo::inline_pgo::inline_decisions_active() {
        let ce = cf.max(1);
        if df >= 8 * ce && size <= 200 && df > 0 {
            // Callee runs >=8x per caller invocation: clearly looped. Only when
            // the callee is hot (measured, not absent) and not recursive.
            return Some(true);
        }
    }
    // Hot caller with a warm call site: inline small callees.
    if caller_hot && site_count > 0 && !site_cold && size <= 32 {
        return Some(true);
    }
    // Cold callee or cold call site: deny medium+ callees (keep tiny
    // helpers — they cost nothing and keep the cold path self-contained).
    if (callee_cold || site_cold) && size > 8 {
        return Some(false);
    }
    if caller_cold && size > 32 {
        return Some(false);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pgo::summary::ProfileSummary;

    #[test]
    fn flat_profile_disables_inline_decisions() {
        // No summary set -> fallback heuristics active.
        crate::pgo::summary::set_summary(ProfileSummary {
            total: 146,
            max: 48,
            second_max: 48, // tied at max -> flat
            hot_threshold: 48,
            cold_threshold: 48,
        });
        assert!(
            !inline_decisions_active(),
            "a flat (tied-max) profile must not drive PGO inlining decisions"
        );
        crate::pgo::summary::set_summary(ProfileSummary {
            total: 2_014_792,
            max: 2_014_727,
            second_max: 64, // dominant hot -> informative
            hot_threshold: 2_014_727,
            cold_threshold: 2_014_727,
        });
        assert!(
            inline_decisions_active(),
            "a dominant-hot profile must drive PGO inlining decisions"
        );
    }
}
