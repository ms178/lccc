//! Wall-clock lap timer for the `[ldtime]` linker diagnostics.
//!
//! The linker drivers report per-phase timings behind `LD_TIME`-style flags.
//! Each report is a *lap*: milliseconds since the previous report, then the
//! clock is restarted.  Modelled as a named type rather than a bare
//! `Instant` local plus a reset statement, because the reset at the end of the
//! final report is dead by construction -- nothing ever reads it again -- and an
//! ad-hoc local cannot express "this write is consumed by the *next* lap" to
//! either a reader or the borrow checker.  With the checkpoint owned by
//! [`LapTimer`] every lap reads the previous checkpoint, so the contract is
//! always exercised, and callers cannot forget to restart the clock.

use std::time::Instant;

/// A restartable wall-clock checkpoint.
pub struct LapTimer {
    last: Instant,
}

impl LapTimer {
    /// Start the first lap at the moment of construction.
    pub fn new() -> Self {
        Self {
            last: Instant::now(),
        }
    }

    /// Milliseconds since the previous lap, in floating point, and start the
    /// next lap.
    pub fn lap_ms(&mut self) -> f64 {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last);
        self.last = now;
        elapsed.as_secs_f64() * 1e3
    }
}

impl Default for LapTimer {
    fn default() -> Self {
        Self::new()
    }
}
