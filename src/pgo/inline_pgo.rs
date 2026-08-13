//! Profile-guided inlining with a bounded, frequency-aware budget.
//!
//! v8: hotness classification is DATA-DRIVEN via the profile summary
//! (`summary::ProfileSummary`, an LLVM `ProfileSummaryInfo` analogue):
//! percentile thresholds computed from the unit's count distribution replace
//! the v6/v7 magic ratios (`relative_frequency >= 0.10/0.05/0.005`).
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

/// v8: summary-driven threshold multiplier. When no summary is available the
/// v7 relative-frequency ratios are used as a fallback.
pub fn inline_threshold_multiplier(caller: &str, callee: &str, p: &ProfileData) -> f64 {
    let cf = p.relative_frequency(caller);
    let df = p.relative_frequency(callee);
    if let Some(s) = crate::pgo::summary::get_summary() {
        let to_count = |r: f64| (r * s.max as f64) as u64;
        let ch = s.is_hot(to_count(cf));
        let dh = s.is_hot(to_count(df));
        let dc = s.is_cold(to_count(df));
        let cc = s.is_cold(to_count(cf));
        if ch && dh {
            1.75
        } else if ch && dc {
            0.60
        } else if cc {
            0.80
        } else {
            1.0
        }
    } else if cf >= 0.10 && df >= 0.05 {
        1.75
    } else if cf >= 0.10 && df < 0.005 {
        0.60
    } else if cf < 0.005 {
        0.80
    } else {
        1.0
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

/// v8: call-site-aware decision. `site_count` is the derived count of the
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
                s.is_hot(df),
                s.is_cold(cf),
                s.is_cold(df),
                site_count > 0 && s.is_hot(site_count),
                site_count > 0 && s.is_cold(site_count),
            ),
            None => {
                // Fallback: v7 relative-frequency ratios.
                let m = p
                    .functions
                    .values()
                    .map(|x| x.total_count)
                    .max()
                    .unwrap_or(0);
                let r = |c: u64| if m == 0 { 0.0 } else { c as f64 / m as f64 };
                (
                    r(cf) >= 0.10,
                    r(df) >= 0.05,
                    r(cf) < 0.005,
                    r(df) < 0.001,
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
    // hot — LLVM HotCallSiteThreshold). Medium hot callees inline.
    if (caller_hot || site_hot) && callee_hot && size <= 48 {
        return Some(true);
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
