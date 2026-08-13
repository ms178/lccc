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

/// The hottest function must be at least this many times hotter than the
/// runner-up for the profile to be considered informative (see
/// `ProfileSummary::has_spread`). A single dominant hot function is the signal
/// that profile-guided decisions have something to optimize; a tie of several
/// "hot" functions is a flat profile where acting on hotness is vacuous.
pub const HOT_DOMINANCE: u64 = 2;

#[derive(Debug, Clone)]
pub struct ProfileSummary {
    pub total: u64,
    pub max: u64,
    /// Second-highest function entry count (0 if only one nonzero function).
    pub second_max: u64,
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
    /// True when the profile reflects genuine hot/cold SEPARATION — the hottest
    /// function is clearly dominant (at least `HOT_DOMINANCE`× the runner-up).
    ///
    /// Percentile thresholds collapse on sparse or flat distributions and are
    /// NOT a reliable separation signal (v10 finding: a flat profile of tied
    /// "hot" functions [48,48,48,1,1] collapses hot==cold==48, and a truly
    /// skewed one [2.0M,64,1] also collapses because the hot count dominates
    /// total). What actually matters for whether profile-driven decisions are
    /// informative is a clear dominant hot path. `has_spread()` is the gate for
    /// profile-driven inlining/layout: on a flat (tied) profile acting on
    /// "hotness" is vacuous and regresses hot paths (adler32 -20%), so the
    /// transforms must be withheld; on a skewed profile they engage.
    pub fn has_spread(&self) -> bool {
        if self.max == 0 {
            return false;
        }
        // A UNIQUE, dominant hot function: the max count must be strictly
        // greater than the runner-up (not tied) and at least HOT_DOMINANCE× it.
        // A tie of several "hot" functions is a flat profile.
        self.max > self.second_max && self.max >= HOT_DOMINANCE * self.second_max.max(1)
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
    // Second-highest count, i.e. the runner-up (for the dominance-based
    // has_spread). This is the SECOND element of the descending sort, so a
    // tie at the max yields second_max == max (the profile is flat). 0 when
    // only one nonzero function exists.
    let mut sorted = counts.clone();
    sorted.sort_unstable_by(|a, b| b.cmp(a));
    let second_max = sorted.get(1).copied().unwrap_or(0);
    if total == 0 {
        return ProfileSummary {
            total,
            max,
            second_max: 0,
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
        second_max,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn counts(v: &[u64]) -> ProfileSummary {
        // Build the summary the same way build_for_unit does (percentile
        // thresholds + second_max) but from an explicit count list, so the
        // test is independent of unit-keying.
        let hot_frac = 0.90;
        let cold_frac = 0.05;
        let total: u64 = v.iter().sum();
        let max = v.iter().copied().max().unwrap_or(0);
        let mut sorted = v.to_vec();
        sorted.sort_unstable_by(|a, b| b.cmp(a));
        let second_max = sorted.get(1).copied().unwrap_or(0);
        if total == 0 {
            return ProfileSummary { total, max, second_max, hot_threshold: 0, cold_threshold: 0 };
        }
        let mut counts = v.to_vec();
        counts.sort_unstable_by(|a, b| b.cmp(a));
        let mut cum = 0u64;
        let mut hot_threshold = 0u64;
        for &c in &counts {
            if c == 0 { break; }
            cum += c;
            if (cum as f64) / (total as f64) >= hot_frac { hot_threshold = c; break; }
        }
        let mut cum2 = 0u64;
        let mut cold_threshold = 0u64;
        for &c in counts.iter().rev() {
            if c == 0 { continue; }
            cum2 += c;
            if (cum2 as f64) / (total as f64) >= cold_frac { cold_threshold = c; break; }
        }
        ProfileSummary { total, max, second_max, hot_threshold, cold_threshold }
    }

    #[test]
    fn flat_tied_hot_is_not_spread() {
        // adler32-style: three tied "hot" functions + a couple cold. Percentile
        // hot==cold==48 (collapse), and max == second_max -> NO dominance.
        let s = counts(&[48, 48, 48, 1, 1]);
        assert!(!s.has_spread(), "tied-hot flat profile must not be informative");
    }

    #[test]
    fn dominant_hot_is_spread() {
        // expat-style: one clearly dominant hot function.
        let s = counts(&[2_014_727, 64, 1]);
        assert!(s.has_spread(), "single dominant hot function must be informative");
    }

    #[test]
    fn two_hot_functions_equal_not_spread() {
        let s = counts(&[100, 100, 1, 1]);
        assert!(!s.has_spread(), "two tied hot functions are still flat");
    }

    #[test]
    fn empty_profile_no_spread() {
        let s = counts(&[]);
        assert!(!s.has_spread());
    }
}
