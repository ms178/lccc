//! Per-translation-unit arithmetic policy shared by optimization and lowering.
//!
//! A worker compiles each TU on one thread. The pass entry resets this policy
//! even at O0; backend address selection must observe the same language contract
//! as IV widening. This is not an environment-controlled optimization switch.
use std::cell::Cell;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Semantics {
    pub(crate) signed_overflow_wraps: bool,
    pub(crate) fp_environment_observable: bool,
}

thread_local! {
    static POLICY: Cell<Semantics> = const { Cell::new(Semantics {
        signed_overflow_wraps: false,
        fp_environment_observable: false,
    }) };
}

pub(crate) fn replace(policy: Semantics) -> Semantics {
    POLICY.with(|cell| cell.replace(policy))
}

pub(crate) fn signed_overflow_wraps() -> bool {
    POLICY.with(|cell| cell.get().signed_overflow_wraps)
}

pub(crate) fn fp_environment_observable() -> bool {
    POLICY.with(|cell| cell.get().fp_environment_observable)
}

#[cfg(test)]
pub(crate) struct TestGuard(Semantics);

#[cfg(test)]
impl TestGuard {
    pub(crate) fn new(policy: Semantics) -> Self {
        Self(replace(policy))
    }
}

#[cfg(test)]
impl Drop for TestGuard {
    fn drop(&mut self) {
        replace(self.0);
    }
}
