//! Test support for process-global and thread-local switch state.
//!
//! Both helpers exist for the same reason: a `#[test]` that flips shared state
//! and restores it by hand leaks that state whenever an assertion fails between
//! the two, and cargo runs tests on a thread pool while REUSING its threads, so
//! the leak lands on tests that have nothing to do with the one that failed.
//! Restoring in `Drop` is the only form that survives a panic.

use std::ffi::OsString;
use std::sync::{Mutex, MutexGuard, OnceLock};

/// The one lock that serializes every test touching the process environment.
///
/// Two modules used to keep their own (`vec_interleave`, `vec_load_sink`), which
/// serialized each module against itself and nothing else: tests from every
/// module share one thread pool, so module A's `set_var` window was still
/// concurrent with module B's `var_os` read.  `getenv` and `setenv` share the
/// `environ` array and glibc's `setenv` may reallocate it — which is exactly why
/// edition 2024 made these calls `unsafe` — so a per-module lock does not
/// discharge the obligation and one process-wide lock does.
static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn env_lock() -> MutexGuard<'static, ()> {
    ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        // A poisoned lock means a test panicked while holding it.  The guard's
        // `Drop` restores the variable regardless, so the environment is not
        // left inconsistent and the remaining tests should still run.
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

thread_local! {
    /// Window nesting depth for THIS thread.  `Mutex` is not reentrant, so a
    /// second guard on the same thread must not take the lock again — it would
    /// deadlock against the first.  Nesting is a real shape (a fixture window
    /// around several per-assertion windows), so depth 0 takes the lock and
    /// every deeper level inherits it.
    static ENV_WINDOW_DEPTH: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The serialized environment window: the process-wide lock at depth 0, an
/// inherited window below it.
struct EnvWindow {
    guard: Option<MutexGuard<'static, ()>>,
}

impl EnvWindow {
    fn acquire() -> Self {
        let depth = ENV_WINDOW_DEPTH.with(|d| {
            let n = d.get();
            d.set(n + 1);
            n
        });
        Self {
            guard: (depth == 0).then(env_lock),
        }
    }
}

impl Drop for EnvWindow {
    fn drop(&mut self) {
        // Decrement before the field drops: the count must say "this thread is
        // outside the window" only once the lock is actually released, and field
        // drops run after `Drop::drop`.
        ENV_WINDOW_DEPTH.with(|d| d.set(d.get() - 1));
    }
}

/// RAII guard over one process-global environment variable.
///
/// This is the audit that five deferred-work markers — each reading "audit that
/// the environment access only happens in single-threaded code" — stood in for.
/// The finding: the access does NOT only happen in single-threaded code, so it
/// has to be
/// serialized and restored rather than argued about —
///
///   * the lock is held for the guard's whole lifetime, so no other test can
///     read or write `environ` while this one is mid-window;
///   * the previous value, including "absent", is restored in `Drop`, so a
///     failing assertion cannot leak the variable into later tests.
///
/// Prefer removing the environment read from the code under test instead.  A
/// kill switch resolved once by the driver and read from per-thread state needs
/// no guard at all (see [`ScopedFlag`], and `TWO_BLOCK_UNROLL_ENABLED` in
/// `loop_unroll`); use this only where reading the environment IS the contract,
/// as it is for `opt_in_env` and `TightLoopMode::from_env`.
pub(crate) struct EnvGuard {
    key: String,
    previous: Option<OsString>,
    // Declared last so it is dropped last: releasing the window before the
    // restore would reopen the race for the restore itself.
    _window: EnvWindow,
}

impl EnvGuard {
    /// Set `key` to `value` for the guard's lifetime.
    pub(crate) fn set(key: &str, value: &str) -> Self {
        Self::with_previous(key, Some(value.to_string()))
    }

    /// Remove `key` for the guard's lifetime.
    pub(crate) fn unset(key: &str) -> Self {
        Self::with_previous(key, None)
    }

    fn with_previous(key: &str, wanted: Option<String>) -> Self {
        let _window = EnvWindow::acquire();
        let previous = std::env::var_os(key);
        // SAFETY: `_window` is held for this guard's lifetime and is the single
        // process-wide serializer for environment mutation, so no other thread
        // can be reading or writing `environ` while this call runs.
        match &wanted {
            Some(value) => unsafe { std::env::set_var(key, value) },
            None => unsafe { std::env::remove_var(key) },
        }
        Self {
            key: key.to_string(),
            previous,
            _window,
        }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        // SAFETY: as in `with_previous` — the guard still holds the window
        // here, because `Drop::drop` runs before the fields are dropped.
        match &self.previous {
            Some(value) => unsafe { std::env::set_var(&self.key, value) },
            None => unsafe { std::env::remove_var(&self.key) },
        }
    }
}

/// RAII restore for a per-thread switch.
///
/// Per-thread state cannot race another test, but cargo reuses its test threads,
/// so a switch left flipped by a panicking assertion would silently change the
/// behaviour of every later test on that thread.  Same shape as [`EnvGuard`],
/// without the lock: the switch is already thread-confined.
pub(crate) struct ScopedFlag {
    set: fn(bool),
    previous: bool,
}

impl ScopedFlag {
    /// Flip the switch addressed by `get`/`set` to `value`, restoring the
    /// previous state on drop.
    pub(crate) fn new(get: fn() -> bool, set: fn(bool), value: bool) -> Self {
        let previous = get();
        set(value);
        Self { set, previous }
    }
}

impl Drop for ScopedFlag {
    fn drop(&mut self) {
        (self.set)(self.previous);
    }
}

#[cfg(test)]
mod tests {
    use super::{EnvGuard, ScopedFlag};
    use std::cell::Cell;
    use std::sync::atomic::{AtomicUsize, Ordering};

    thread_local! {
        static FLAG: Cell<bool> = const { Cell::new(true) };
    }

    /// What the code under test would see for `key`.  Every value these tests
    /// set is valid UTF-8, so reading it as a `String` keeps the assertions
    /// about behaviour rather than about `OsStr` conversions.
    fn value(key: &str) -> Option<String> {
        std::env::var_os(key).map(|v| v.to_string_lossy().into_owned())
    }

    fn get_flag() -> bool {
        FLAG.with(Cell::get)
    }

    fn set_flag(value: bool) {
        FLAG.with(|cell| cell.set(value));
    }

    #[test]
    fn env_guard_restores_the_previous_value() {
        const KEY: &str = "CCC_TEST_SUPPORT_GUARD_PREVIOUS";
        // Nested on purpose: the window is reentrant for its own thread, which
        // is what makes "a fixture window around per-assertion windows" usable
        // and what makes the restore-to-value arm testable at all.
        let outer = EnvGuard::set(KEY, "outer");
        assert_eq!(value(KEY).as_deref(), Some("outer"));
        {
            let _inner = EnvGuard::set(KEY, "inner");
            assert_eq!(value(KEY).as_deref(), Some("inner"));
        }
        assert_eq!(
            value(KEY).as_deref(),
            Some("outer"),
            "the inner guard did not restore the value it found"
        );
        drop(outer);
        assert_eq!(value(KEY), None, "the outer guard left the variable set");
    }

    #[test]
    fn env_guard_restores_absence() {
        const KEY: &str = "CCC_TEST_SUPPORT_GUARD_ABSENT";
        let _outer = EnvGuard::unset(KEY);
        assert_eq!(value(KEY), None);
        {
            let _inner = EnvGuard::set(KEY, "1");
            assert_eq!(value(KEY).as_deref(), Some("1"));
        }
        assert_eq!(
            value(KEY),
            None,
            "a variable that was absent came back as present"
        );
    }

    #[test]
    fn the_guard_restores_even_when_the_body_panics() {
        // The failure mode the deferred-work markers stood in for: an assertion
        // between set and restore.  `catch_unwind` runs the panic here so the
        // rest of the suite still sees a clean environment.
        const KEY: &str = "CCC_TEST_SUPPORT_GUARD_PANIC";
        let _outer = EnvGuard::unset(KEY);
        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = EnvGuard::set(KEY, "leaked?");
            panic!("the body failed between set and restore");
        }));
        assert!(panicked.is_err(), "the body did not panic");
        assert_eq!(
            value(KEY),
            None,
            "Drop did not run, so the variable leaked out of the panicking body"
        );
    }

    #[test]
    fn scoped_flag_restores_the_switch() {
        assert!(get_flag());
        {
            let _g = ScopedFlag::new(get_flag, set_flag, false);
            assert!(!get_flag());
        }
        assert!(get_flag(), "the switch was left flipped");
        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _g = ScopedFlag::new(get_flag, set_flag, false);
            panic!("failed with the switch off");
        }));
        assert!(panicked.is_err());
        assert!(get_flag(), "Drop did not restore the switch after a panic");
    }

    #[test]
    fn the_lock_serializes_concurrent_guard_holders() {
        // Not a proof of mutual exclusion (that is the standard library's), but
        // a check that the guard really does hold the lock for its lifetime: if
        // it released early, two threads could be inside the window at once and
        // the counter would exceed 1.
        static INSIDE: AtomicUsize = AtomicUsize::new(0);
        static OVERLAP: AtomicUsize = AtomicUsize::new(0);
        const KEY: &str = "CCC_TEST_SUPPORT_GUARD_LOCK";
        // No outer guard here: the window is reentrant for its OWN thread only,
        // so holding one while the scoped threads take theirs would deadlock
        // against itself -- which is itself the reason the depth counter is
        // thread-local rather than a plain "is locked" flag.
        std::thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| {
                    for _ in 0..200 {
                        let _g = EnvGuard::set(KEY, "1");
                        let now = INSIDE.fetch_add(1, Ordering::SeqCst) + 1;
                        if now > 1 {
                            OVERLAP.fetch_add(1, Ordering::SeqCst);
                        }
                        std::hint::spin_loop();
                        INSIDE.fetch_sub(1, Ordering::SeqCst);
                    }
                });
            }
        });
        // The threads leave the variable set: each guard restores what it found,
        // and after the first iteration what it finds is the value itself.  Take
        // it back out with a guard of its own so nothing leaks into the suite.
        let _cleanup = EnvGuard::unset(KEY);
        assert_eq!(value(KEY), None, "the cleanup guard left the variable set");
        assert_eq!(
            OVERLAP.load(Ordering::SeqCst),
            0,
            "two guards were inside the environment window at the same time"
        );
    }
}
