//! Language semantics carried with IR, never ambient worker-thread state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PassPolicy {
    pub wrap_signed_overflow: bool,
    pub fp_environment_observable: bool,
}

impl PassPolicy {
    pub const fn new(wrap_signed_overflow: bool, fp_environment_observable: bool) -> Self {
        Self {
            wrap_signed_overflow,
            fp_environment_observable,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn flags_are_independent() {
        for wrap in [false, true] {
            for fenv in [false, true] {
                let policy = PassPolicy::new(wrap, fenv);
                assert_eq!(policy.wrap_signed_overflow, wrap);
                assert_eq!(policy.fp_environment_observable, fenv);
            }
        }
    }
}
