//! Profile summary (LLVM `ProfileSummaryInfo` analogue).
//!
//! Data-driven hot/cold classification. LLVM derives hot/cold thresholds
//! from percentiles of the profile count distribution (ProfileSummaryInfo:
//! `computeThreshold(PercentileCutoff)` walks the sorted counts until the
//! cumulative execution share reaches the cutoff, and `isHotCallSite`/
//! `isColdCallSite` apply those thresholds to per-call-site block
//! frequencies from BFI). We do exactly that:
//!
//!   * `hot_threshold`  = the smallest count such that counts >= it carry at
//!                        least `LCCC_PGO_HOT_FRAC` (default 0.90) of the
//!                        unit's total execution;
//!   * `cold_threshold` = the largest count such that counts <= it carry at
//!                        least `LCCC_PGO_COLD_FRAC` (default 0.05) of total.
//!
//! This replaces the v6/v7 magic ratios (`total_count*100 >= max`,
//! `relative_frequency >= 0.10/0.05/0.005`, `n > 1000`) with thresholds that
//! adapt to the actual count distribution: a unit dominated by one hot
//! function gets a high threshold, a flat unit gets a low one.
use crate::common::fx_hash::FxHashMap;
use crate::pgo::profile::unit_hash;
use crate::pgo::ProfileData;

#[derive(Debug, Clone)]
pub struct ProfileSummary {
    pub total: u64,
    pub max: u64,
    pub hot_threshold: u64,
    pub cold_threshold: u64,
}

impl ProfileSummary {
    pub fn is_hot(&self, c: u64) -> bool {
        c > 0 && self.hot_threshold > 0 && c >= self.hot_threshold
    }
    pub fn is_cold(&self, c: u64) -> bool {
        self.cold_threshold > 0 && c < self.cold_threshold
    }
    /// Normalized hotness in [0,1]: c / max.
    pub fn hotness(&self, c: u64) -> f64 {
        if self.max == 0 {
            0.0
        } else {
            c as f64 / self.max as f64
        }
    }
}

/// Build the summary for one translation unit from the loaded profile.
/// Counts are per-function entry counts (unit-scoped). The thresholds are
/// computed from the cumulative execution distribution — see the module doc.
pub fn build_for_unit(p: &ProfileData, unit: &str) -> ProfileSummary {
    let hot_frac: f64 = std::env::var("LCCC_PGO_HOT_FRAC")
        .ok()
        .and_then(|x| x.parse::<f64>().ok())
        .unwrap_or(0.90)
        .clamp(0.01, 1.0);
    let cold_frac: f64 = std::env::var("LCCC_PGO_COLD_FRAC")
        .ok()
        .and_then(|x| x.parse::<f64>().ok())
        .unwrap_or(0.05)
        .clamp(0.001, 0.5);
    let prefix = format!("{:016x}::", unit_hash(unit));
    let mut counts: Vec<u64> = p
        .functions
        .iter()
        .filter(|(k, _)| k.starts_with(&prefix))
        .map(|(_, f)| f.total_count)
        .collect();
    let total: u64 = counts.iter().sum();
    let max = counts.iter().copied().max().unwrap_or(0);
    if total == 0 {
        return ProfileSummary {
            total,
            max,
            hot_threshold: 0,
            cold_threshold: 0,
        };
    }
    // Hot: walk from the largest count downward until the cumulative share
    // of execution reaches hot_frac; the count at that point is the
    // threshold (LLVM ProfileSummaryInfo::computeThreshold).
    counts.sort_unstable_by(|a, b| b.cmp(a));
    let mut cum = 0u64;
    let mut hot_threshold = 0u64;
    for &c in &counts {
        if c == 0 {
            break;
        }
        cum += c;
        if (cum as f64) / (total as f64) >= hot_frac {
            hot_threshold = c;
            break;
        }
    }
    // Cold: walk from the smallest nonzero count upward until the share
    // reaches cold_frac; counts strictly below that point are cold.
    let mut cum2 = 0u64;
    let mut cold_threshold = 0u64;
    for &c in counts.iter().rev() {
        if c == 0 {
            continue;
        }
        cum2 += c;
        if (cum2 as f64) / (total as f64) >= cold_frac {
            cold_threshold = c;
            break;
        }
    }
    ProfileSummary {
        total,
        max,
        hot_threshold,
        cold_threshold,
    }
}

/// Stashed summary for the unit currently being compiled (filled by the
/// driver right after profile activation; recomputed on demand otherwise).
static SUMMARY: std::sync::LazyLock<std::sync::Mutex<Option<ProfileSummary>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(None));

pub fn set_summary(s: ProfileSummary) {
    *SUMMARY.lock().unwrap() = Some(s);
}
pub fn get_summary() -> Option<ProfileSummary> {
    SUMMARY.lock().unwrap().clone()
}
/// Build (and cache) the summary for `unit`.
pub fn summary_for_unit(p: &ProfileData, unit: &str) -> ProfileSummary {
    if let Some(s) = get_summary() {
        return s;
    }
    let s = build_for_unit(p, unit);
    set_summary(s.clone());
    s
}

/// Per-unit count lookup helpers used by consumers that only have the unit
/// string (pre-pass consumers have no IrFunction yet).
pub fn entry_count_for<'a>(p: &'a ProfileData, unit: &str, name: &str) -> u64 {
    crate::pgo::profile::get_for_unit(p, unit, name)
        .map(|f| f.total_count)
        .unwrap_or(0)
}
pub fn edge_count_for<'a>(
    p: &'a ProfileData,
    unit: &str,
    name: &str,
    src: u32,
    dst: u32,
) -> u64 {
    crate::pgo::profile::get_for_unit(p, unit, name)
        .map(|f| f.edge_count(src, dst))
        .unwrap_or(0)
}
pub fn block_count_for<'a>(p: &'a ProfileData, unit: &str, name: &str, label: u32) -> u64 {
    crate::pgo::profile::get_for_unit(p, unit, name)
        .map(|f| f.block_count(crate::ir::reexports::BlockId(label)))
        .unwrap_or(0)
}
pub fn _unused(_: FxHashMap<(), ()>) {}
