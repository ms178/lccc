//! Round-cap observability for bounded fixpoint loops (T5 in the vellvm/vir
//! transplant register, `engineering/FOLLOWUP-2026-10-18-vellvm-vir-transplant.md`).
//!
//! Several passes iterate `for _ in 0..MAX_ROUNDS { let n = round(); if n == 0
//! { break } }`, and the inliner runs an ordinary and a second-pass round
//! budget. Stopping at a budget is sound -- every round is a complete,
//! verified rewrite on its own, so a bound hit only leaves optimization
//! opportunities unclaimed -- but the exit was invisible. A site calls
//! [`note_cap_hit`] exactly when its budget ran out while the last round still
//! changed the IR. [`report`] drains the process-wide counters and prints the
//! nonzero ones when the pipeline runs with `CCC_TIME_PASSES`.
//!
//! Design notes:
//! * The counters are relaxed atomics touched only on the exit path, so hot
//!   loops pay nothing.
//! * Tests use a private [`CapCounters`] instance. The global counters are
//!   shared by every test that runs a real pass, so a test that read them
//!   would race with unrelated passes and fail intermittently.
//! * No environment reads here: `tests/regression/check_env_test_hygiene.sh`
//!   ratchets the pass pipeline's environment reads downward, and the switch
//!   is read once in `run_passes`.

use std::sync::atomic::{AtomicU64, Ordering};

/// Every budgeted loop that reports a cap hit. Adding a site means adding a
/// variant here, a `name`, and exactly one [`note_cap_hit`] call at the exit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Site {
    LoopCarriedForward,
    BackedgePre,
    ReassocLatency,
    BoolThread,
    LoopPreheader,
    InlineOrdinaryRounds,
    InlineSecondPassRounds,
}

impl Site {
    /// Every site, in declaration order (the order `report` prints).
    pub(crate) const ALL: [Site; 7] = [
        Site::LoopCarriedForward,
        Site::BackedgePre,
        Site::ReassocLatency,
        Site::BoolThread,
        Site::LoopPreheader,
        Site::InlineOrdinaryRounds,
        Site::InlineSecondPassRounds,
    ];

    /// Stable name used in `report` output and in the regression gate.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Site::LoopCarriedForward => "loop_carried_forward",
            Site::BackedgePre => "backedge_pre",
            Site::ReassocLatency => "reassoc_latency",
            Site::BoolThread => "bool_thread",
            Site::LoopPreheader => "loop_preheader",
            Site::InlineOrdinaryRounds => "inline_ordinary_rounds",
            Site::InlineSecondPassRounds => "inline_second_pass_rounds",
        }
    }
}

/// One relaxed counter per [`Site`].
pub(crate) struct CapCounters {
    hits: [AtomicU64; Site::ALL.len()],
}

impl CapCounters {
    pub(crate) const fn new() -> Self {
        Self {
            hits: [const { AtomicU64::new(0) }; Site::ALL.len()],
        }
    }

    /// Count one cap hit at `site`.
    pub(crate) fn note(&self, site: Site) {
        self.hits[site as usize].fetch_add(1, Ordering::Relaxed);
    }

    /// Cumulative hits recorded for `site`.
    pub(crate) fn get(&self, site: Site) -> u64 {
        self.hits[site as usize].load(Ordering::Relaxed)
    }

    /// Return the hits recorded for `site` and reset them to zero.
    pub(crate) fn take(&self, site: Site) -> u64 {
        self.hits[site as usize].swap(0, Ordering::Relaxed)
    }
}

static GLOBAL: CapCounters = CapCounters::new();

/// Record that `site` stopped because its round budget ran out while the last
/// round still changed the IR.
pub(crate) fn note_cap_hit(site: Site) {
    GLOBAL.note(site);
}

/// One `report` line for `site`, or `None` when it has never hit its cap.
pub(crate) fn report_line(site: Site, count: u64) -> Option<String> {
    (count > 0).then(|| format!("[PASS] round_cap {}: {} hit(s)", site.name(), count))
}

/// Print the hits recorded since the previous call, one line per site that
/// hit its cap, and reset the counters. Called only under `CCC_TIME_PASSES`.
/// Resetting makes each module's report describe that module alone.
pub(crate) fn report() {
    for site in Site::ALL {
        if let Some(line) = report_line(site, GLOBAL.take(site)) {
            eprintln!("{line}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_unique_and_match_declaration_order() {
        let names: Vec<&str> = Site::ALL.iter().map(|s| s.name()).collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), names.len(), "duplicate site name: {names:?}");
        for (i, site) in Site::ALL.iter().enumerate() {
            assert_eq!(*site as usize, i, "Site::ALL out of declaration order");
        }
    }

    #[test]
    fn counters_are_per_site_and_take_resets() {
        let c = CapCounters::new();
        c.note(Site::ReassocLatency);
        c.note(Site::ReassocLatency);
        assert_eq!(c.get(Site::ReassocLatency), 2);
        assert_eq!(c.get(Site::BackedgePre), 0);
        assert_eq!(c.take(Site::ReassocLatency), 2);
        assert_eq!(c.get(Site::ReassocLatency), 0);
        assert_eq!(c.take(Site::ReassocLatency), 0);
    }

    #[test]
    fn counters_are_exact_under_concurrent_notes() {
        let c = CapCounters::new();
        std::thread::scope(|s| {
            for _ in 0..4 {
                s.spawn(|| {
                    for _ in 0..1000 {
                        c.note(Site::InlineSecondPassRounds);
                    }
                });
            }
        });
        assert_eq!(c.get(Site::InlineSecondPassRounds), 4000);
    }

    #[test]
    fn report_line_is_silent_at_zero_and_names_the_site() {
        assert_eq!(report_line(Site::BoolThread, 0), None);
        assert_eq!(
            report_line(Site::BoolThread, 3).as_deref(),
            Some("[PASS] round_cap bool_thread: 3 hit(s)")
        );
    }
}
