//! Dual-layout metric peepholes: pair scheduling shared by every backend.
//!
//! At `-O1` and above, `block_layout`'s static-chain exploration cannot predict
//! whether its reordering helps (measured: -42 instructions on `linux_rbtree`,
//! +38 on `strlen_bench`), so codegen settles it empirically: emit the chained
//! block order, emit the restored original order, run the backend's *text
//! peephole* over each, count instructions, keep the smaller, re-emit the
//! winner.  The two candidates are independent fragments, which makes the pair
//! the coarsest unit of codegen parallelism that needs no synchronisation and
//! the finest one that is still profitable.
//!
//! This module owns that scheduling decision for **all four backends** (x86-64,
//! i686, AArch64, RISC-V): every one of them runs a real text peephole in
//! `ArchCodegen::peephole_for_metric`, so every one of them paid the two pass
//! mixes serially.  It is a *latency* change only.  The pipeline is
//! text-in/text-out, the per-arch policy a backend needs is captured by value
//! or by shared reference at the call site, and the emitted assembly is
//! byte-identical in both scheduling modes — asserted, not assumed, by
//! `tests/regression/check_peephole_pair_parallel.py` for each architecture.
//!
//! It is deliberately *not* a throughput optimization: a `make -j N` build
//! already saturates the host, and this adds at most one runnable thread at a
//! time, only while a pair above [`MIN_PARALLEL_PAIR_BYTES`] is measured.
//! `LCCC_PEEPHOLE_PAIR_PARALLEL=0` restores serial behaviour for every backend.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Combined candidate bytes below which a pair stays serial.
///
/// Calibrated from measurement, not folklore.  A scoped spawn+join costs
/// 47-59 µs on a current x86-64 host (10 000-iteration microbenchmark), while
/// the x86-64 peephole pipeline sustains roughly 0.4 bytes/µs, so splitting a
/// pair repays the spawn once a *candidate* holds ~130 bytes.  The other
/// backends' pass mixes are smaller but the same order of magnitude.
///
/// 1 KiB is therefore an 8x safety floor over the measured crossover rather
/// than a tuned optimum: it keeps degenerate fragments (a two-block stub) off
/// the scheduler while leaving every pair the corpus actually produces
/// splittable.  The corpus distribution is why the floor matters — 34 pairs
/// across the `-O1` x86 benchmark programs range 1 972 B - 16 063 B with a
/// 5 714 B median, so a higher floor silently rejects most of them (an initial
/// 8 KiB value rejected ~70 %, measured as 18.5 % of `glibc_memcmp` wall time
/// by `scripts/measure_peephole_pair.py --arms serial,auto,forced`).
pub(crate) const MIN_PARALLEL_PAIR_BYTES: usize = 1024;

/// Which half of a dual-layout pair a metric peephole is processing.
///
/// Diagnostic only.  The x86-64 debug instruments (`CCC_DUMP_ASM`,
/// `CCC_PEEPHOLE_TRACE`) name their output files from it so the two candidates
/// cannot overwrite each other — they did, and once the pair ran concurrently
/// the survivor became nondeterministic on exactly the functions people bisect
/// miscompiles on.  No pass reads it, and a backend without instruments simply
/// never observes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MetricCandidate {
    Chain,
    Original,
}

impl MetricCandidate {
    pub(crate) fn suffix(self) -> &'static str {
        match self {
            Self::Chain => "-chain",
            Self::Original => "-orig",
        }
    }
}

thread_local! {
    static METRIC_CANDIDATE: std::cell::Cell<Option<MetricCandidate>> =
        const { std::cell::Cell::new(None) };
}

/// Label the fragment a metric peephole is processing, restoring the previous
/// label on both normal return and unwind.  Thread-local by construction, so a
/// concurrent candidate cannot see or clear the other's label.
pub(crate) fn tag_metric_candidate<R>(candidate: MetricCandidate, job: impl FnOnce() -> R) -> R {
    struct Untag(Option<MetricCandidate>);
    impl Drop for Untag {
        fn drop(&mut self) {
            METRIC_CANDIDATE.with(|slot| slot.set(self.0));
        }
    }
    let _untag = Untag(METRIC_CANDIDATE.with(|slot| slot.replace(Some(candidate))));
    job()
}

/// `-chain`/`-orig` suffix for an instrument filename; empty outside a pair.
pub(crate) fn metric_candidate_suffix() -> &'static str {
    METRIC_CANDIDATE
        .with(std::cell::Cell::get)
        .map_or("", MetricCandidate::suffix)
}

/// `LCCC_PEEPHOLE_PAIR_PARALLEL` — kill switch and A/B lever, resolved once.
///
/// `0`/`off`/`false` never spawns; `1`/`on`/`true` always spawns on a
/// multi-thread host, ignoring the size gate (that is what makes the parallel
/// branch reachable in a test on any machine); unset/`auto`/anything else
/// keeps the size-and-host heuristic.  Unrecognised values never abort: a typo
/// degrades to the default rather than failing a build.
fn pair_override() -> Option<bool> {
    static OVERRIDE: OnceLock<Option<bool>> = OnceLock::new();
    *OVERRIDE.get_or_init(|| {
        let raw = std::env::var("LCCC_PEEPHOLE_PAIR_PARALLEL").ok()?;
        let raw = raw.trim();
        if raw == "1" || raw.eq_ignore_ascii_case("true") || raw.eq_ignore_ascii_case("on") {
            return Some(true);
        }
        if raw == "0" || raw.eq_ignore_ascii_case("false") || raw.eq_ignore_ascii_case("off") {
            return Some(false);
        }
        None
    })
}

/// Host capability, resolved once: `available_parallelism` is a `sysconf` plus
/// a cgroup read, and a pair decision happens once per explored function.
fn host_is_multi_threaded() -> bool {
    static MULTI: OnceLock<bool> = OnceLock::new();
    *MULTI.get_or_init(|| std::thread::available_parallelism().is_ok_and(|n| n.get() > 1))
}

/// Whether the per-pair telemetry line is wanted, resolved once.  Cached for
/// the same reason as the knob: a pair decides once per explored function, and
/// the answer cannot change during a compilation.
fn telemetry_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("CCC_TIME_PEEPHOLE").is_some())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PairMode {
    Serial,
    Parallel,
}

impl PairMode {
    fn label(self) -> &'static str {
        match self {
            Self::Serial => "serial",
            Self::Parallel => "parallel",
        }
    }
}

/// The scheduling decision, as a pure function of its three inputs.
///
/// Kept pure and separate from [`pair_override`]/[`host_is_multi_threaded`] so
/// the whole policy matrix is testable without touching process environment or
/// host topology — the cached env lookups make an environment-driven test
/// order-dependent, and `cargo test` runs tests concurrently in one process.
fn pair_mode_for(forced: Option<bool>, multi_thread: bool, combined_bytes: usize) -> PairMode {
    let spawn = match forced {
        Some(forced) => forced,
        None => combined_bytes >= MIN_PARALLEL_PAIR_BYTES,
    };
    // A forced-parallel request still cannot spawn usefully on one hardware
    // thread: the worker would timeshare the caller it is meant to overlap.
    if spawn && multi_thread {
        PairMode::Parallel
    } else {
        PairMode::Serial
    }
}

/// Pair scheduling counters.  They are the only observable proof that the
/// parallel branch executed: by contract both modes emit byte-identical text,
/// so no assembly-level assertion can distinguish them.
static PAIRS_PARALLEL: AtomicUsize = AtomicUsize::new(0);
static PAIRS_SERIAL: AtomicUsize = AtomicUsize::new(0);
static PAIRS_DEDUPED: AtomicUsize = AtomicUsize::new(0);

const RELAXED: Ordering = Ordering::Relaxed;

/// `(parallel, serial, deduped)` pair counts since process start.  They are
/// process-global, so tests must assert on deltas, never on absolutes.
pub(crate) fn pair_counters() -> (usize, usize, usize) {
    (
        PAIRS_PARALLEL.load(RELAXED),
        PAIRS_SERIAL.load(RELAXED),
        PAIRS_DEDUPED.load(RELAXED),
    )
}

/// Run two independent jobs in `mode`, tagging each as its candidate.
///
/// The worker takes the first job and the caller keeps the second.  The two
/// candidates are the same function under two block orders, so their work is
/// near-equal by construction and the assignment order cannot skew the critical
/// path; keeping the simpler rule is what makes the result ordering (argument
/// order, always) obvious by inspection.  At most one worker exists per pair,
/// pairs never nest, and a worker panic is resumed on the caller, so the
/// failure is identical to the serial path.
fn run_pair_tagged<T, R>(
    mode: PairMode,
    first: T,
    second: T,
    apply: impl Fn(T) -> R + Copy + Send,
) -> (R, R)
where
    T: Send,
    R: Send,
{
    if mode == PairMode::Serial {
        return (
            tag_metric_candidate(MetricCandidate::Chain, || apply(first)),
            tag_metric_candidate(MetricCandidate::Original, || apply(second)),
        );
    }
    std::thread::scope(|scope| {
        let worker =
            scope.spawn(move || tag_metric_candidate(MetricCandidate::Chain, || apply(first)));
        let second_result = tag_metric_candidate(MetricCandidate::Original, || apply(second));
        let first_result = match worker.join() {
            Ok(result) => result,
            Err(payload) => std::panic::resume_unwind(payload),
        };
        (first_result, second_result)
    })
}

/// Split-check counters plus the optional telemetry line, then run the pair.
fn schedule<R>(arch: &'static str, sizes: (usize, usize), run: impl FnOnce(PairMode) -> R) -> R {
    // Saturating add: each length is below `isize::MAX`, so the sum cannot wrap
    // on any supported target; saturating keeps the comparison total without
    // introducing a panic path.
    let combined = sizes.0.saturating_add(sizes.1);
    let mode = pair_mode_for(pair_override(), host_is_multi_threaded(), combined);
    match mode {
        PairMode::Parallel => PAIRS_PARALLEL.fetch_add(1, RELAXED),
        PairMode::Serial => PAIRS_SERIAL.fetch_add(1, RELAXED),
    };
    if telemetry_enabled() {
        let (parallel, serial, deduped) = pair_counters();
        eprintln!(
            "[pair] arch={arch} chain={}B orig={}B mode={} parallel={parallel} serial={serial} deduped={deduped}",
            sizes.0,
            sizes.1,
            mode.label()
        );
    }
    run(mode)
}

/// Optimize two independent dual-layout candidates, concurrently when the pair
/// is worth splitting.
///
/// `arch` only labels the telemetry line.  `apply` must be the backend's metric
/// peephole as a pure `text -> text` function: every backend's is, which is
/// what makes the concurrent path output-neutral.
pub(crate) fn run_metric_pair(
    arch: &'static str,
    first: String,
    second: String,
    apply: impl Fn(String) -> String + Copy + Send,
) -> (String, String) {
    // Identical fragments cannot produce different results (the pipeline is a
    // pure function of text plus the caller's policy), so the second pass is
    // skipped outright.  `String` equality exits at the first differing byte —
    // for two block orders that is the first divergent instruction — so the
    // check cannot cost more than a fraction of the pass mix it can skip.
    if first == second {
        PAIRS_DEDUPED.fetch_add(1, RELAXED);
        let optimized = tag_metric_candidate(MetricCandidate::Chain, || apply(first));
        return (optimized.clone(), optimized);
    }
    let sizes = (first.len(), second.len());
    schedule(arch, sizes, move |mode| {
        run_pair_tagged(mode, first, second, apply)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole scheduling matrix: both sides of the size gate, both forced
    /// modes, and the single-thread host that must never spawn — a worker
    /// there would timeshare the caller it is meant to overlap.
    #[test]
    fn pair_mode_matrix_honours_gate_override_and_host() {
        assert_eq!(
            pair_mode_for(None, true, MIN_PARALLEL_PAIR_BYTES - 1),
            PairMode::Serial
        );
        assert_eq!(
            pair_mode_for(None, true, MIN_PARALLEL_PAIR_BYTES),
            PairMode::Parallel,
            "the gate is inclusive"
        );
        assert_eq!(pair_mode_for(None, false, usize::MAX), PairMode::Serial);
        assert_eq!(pair_mode_for(Some(true), true, 0), PairMode::Parallel);
        assert_eq!(pair_mode_for(Some(true), false, 0), PairMode::Serial);
        assert_eq!(
            pair_mode_for(Some(false), true, usize::MAX),
            PairMode::Serial,
            "the kill switch wins over size and host"
        );
    }

    /// The seam that lets every host test the concurrent branch: both jobs run
    /// exactly once, and results keep argument order regardless of which side
    /// won the race.
    #[test]
    fn run_pair_runs_both_jobs_once_and_keeps_argument_order() {
        for mode in [PairMode::Serial, PairMode::Parallel] {
            let executed = AtomicUsize::new(0);
            let apply = |input: usize| {
                executed.fetch_add(1, RELAXED);
                input * 10
            };
            assert_eq!(
                run_pair_tagged(mode, 1usize, 2usize, apply),
                (10, 20),
                "{mode:?} keeps argument order"
            );
            assert_eq!(executed.load(RELAXED), 2, "{mode:?} ran both jobs");
        }
    }

    /// A panicking candidate must abort the pair with its own payload, exactly
    /// like the serial path: `join().unwrap()` would have flattened the payload
    /// and added a panic site.
    #[test]
    fn worker_panic_is_resumed_on_the_caller() {
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run_pair_tagged(PairMode::Parallel, (), (), |()| -> usize {
                panic!("pair payload")
            })
        }));
        match outcome {
            Ok(values) => panic!("worker panic must surface on the caller, got {values:?}"),
            Err(payload) => assert_eq!(payload.downcast_ref::<&str>(), Some(&"pair payload")),
        }
    }

    /// The candidate label is scoped, nests, and is restored on unwind — the
    /// property that keeps a concurrent candidate from clearing the other's.
    #[test]
    fn metric_candidate_label_is_scoped_and_restored_on_unwind() {
        assert!(metric_candidate_suffix().is_empty(), "untagged by default");
        assert_eq!(
            tag_metric_candidate(MetricCandidate::Chain, metric_candidate_suffix),
            "-chain"
        );
        let nested = tag_metric_candidate(MetricCandidate::Original, || {
            tag_metric_candidate(MetricCandidate::Chain, metric_candidate_suffix)
        });
        assert_eq!(nested, "-chain", "the inner label wins while it runs");
        assert!(
            metric_candidate_suffix().is_empty(),
            "both scopes restored their predecessor"
        );
        let unwound = std::panic::catch_unwind(|| {
            tag_metric_candidate(MetricCandidate::Original, || panic!("unwind the label"))
        });
        assert!(unwound.is_err());
        assert!(
            metric_candidate_suffix().is_empty(),
            "an unwinding job still restores the label"
        );
    }

    /// Both halves of a pair must be tagged, in argument order, in both modes:
    /// that label is what keeps a backend's instruments from colliding.
    #[test]
    fn every_side_of_a_pair_is_tagged_with_its_own_candidate() {
        for mode in [PairMode::Serial, PairMode::Parallel] {
            let first = |text: String| format!("{}{}", metric_candidate_suffix(), text);
            let (a, b) = run_pair_tagged(mode, "x".to_string(), "y".to_string(), first);
            assert_eq!(a, "-chainx", "{mode:?}");
            assert_eq!(b, "-origy", "{mode:?}");
        }
    }

    /// Identical candidates cannot produce different results, so the second
    /// pass is skipped — and the counter proves the shortcut, not just the
    /// output.
    #[test]
    fn identical_candidates_are_processed_once() {
        let before = pair_counters().2;
        let apply = |text: String| text.to_uppercase();
        let (first, second) =
            run_metric_pair("test", "same".to_string(), "same".to_string(), apply);
        assert_eq!(first, "SAME");
        assert_eq!(second, "SAME");
        assert!(
            pair_counters().2 > before,
            "the dedupe counter must advance (deltas only: it is process-global)"
        );
    }
}
