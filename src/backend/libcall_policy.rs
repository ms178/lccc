//! The backend half of the A14 rule: a constant-size `memcpy`/`memset` call
//! may be expanded inline only while the callee still is the builtin.
//!
//! Two facts decide that, and both come from outside this module:
//!
//! * **The CLI.**  `-fno-builtin`, `-ffreestanding` and `-fno-builtin-<fn>`
//!   withdraw builtin status; GCC 16.2 and Clang 23.1 measured: with any of
//!   them, `memset(b, 7, 8)` stays a *call* (the `__memset_chk` fortified
//!   form does not — those stay inlined under all four flag combinations,
//!   and the gate asserts that row so a later change cannot start gating
//!   them silently).  Published by the driver right after the tuning row
//!   (`crate::driver::pipeline`, `set`).
//! * **The module.**  A translation unit that *defines* `memset` (glibc's
//!   own string routines do) makes the call resolve to that definition.  The
//!   expansion used to bake the library's fill anyway: with a TU-defined
//!   `memset` that ignores `c`, `memset(b, 7, 8)` grew a `movabsq
//!   $0x707070707070707` store instead of the call (A14b — measured: lccc
//!   pre-fix printed 7 where GCC printed 0; the pre-fix `memset` body even
//!   contained `call memset@PLT` with the same `d`/`n`, i.e. unconditional
//!   self-recursion).  Published at codegen entry
//!   (`generation::collect_symbol_sets`, `set_defined`), from the same
//!   collector the pass-side gate uses (`common::builtin::defined_symbols`).
//!
//! Callers of the predicate, all const-size expansion paths:
//! `inline_memcpy_len` and `inline_memset_const_len` (via
//! `x86_inline_memset_len`), which are consulted by the plain-call dispatch
//! (`generation.rs`), the MachInst typed-call gate
//! (`x86/codegen/emit.rs::supports_inline_memcpy_call`) and the prologue /
//! regalloc eligibility checks — one choke point, no bypass.
//!
//! `__memset_chk` / `__memcpy_chk` arms are deliberately *not* gated: both
//! reference compilers keep inlining them under every withdrawal spelling.
//! With `-D_FORTIFY_SOURCE=2 -fno-builtin` lccc therefore emits a plain
//! `memset` call where GCC keeps a fortified fill (lccc's fortify rewrite is
//! itself builtin-gated) — same observable semantics, strictly weaker
//! assumptions, recorded in the regression gate.

use crate::common::builtin::BuiltinPolicy;
use crate::common::fx_hash::FxHashSet;
use std::sync::OnceLock;
#[cfg(test)]
use std::sync::{Mutex, MutexGuard};

static ACTIVE: OnceLock<std::sync::RwLock<BuiltinPolicy>> = OnceLock::new();
static DEFINED: OnceLock<std::sync::RwLock<Option<FxHashSet<String>>>> = OnceLock::new();

fn active_cell() -> &'static std::sync::RwLock<BuiltinPolicy> {
    ACTIVE.get_or_init(|| std::sync::RwLock::new(BuiltinPolicy::default()))
}

fn defined_cell() -> &'static std::sync::RwLock<Option<FxHashSet<String>>> {
    DEFINED.get_or_init(|| std::sync::RwLock::new(None))
}

/// Publish the CLI's builtin-withdrawal state for this compilation.
pub(crate) fn set(policy: BuiltinPolicy) {
    *active_cell().write().unwrap_or_else(|e| e.into_inner()) = policy;
}

/// Publish the module's defined symbols, collected by
/// [`crate::common::builtin::defined_symbols`] — the same collector the
/// pass-side gate snapshots.
pub(crate) fn set_defined(defined: FxHashSet<String>) {
    *defined_cell().write().unwrap_or_else(|e| e.into_inner()) = Some(defined);
}

/// May the backend assume `name` still denotes the builtin?
///
/// Fail-closed before publication: a const-size expansion can only happen
/// once the driver has published the policy and a module has been published,
/// so an early query is a programming error, not a legitimate "permissive"
/// state — and permissive is exactly the wrong default for a gate that
/// protects against self-recursion.
pub(crate) fn may_assume_builtin(name: &str) -> bool {
    let policy = active_cell().read().unwrap_or_else(|e| e.into_inner());
    if policy.withdrawn(name) {
        return false;
    }
    drop(policy);
    match defined_cell()
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
    {
        Some(defined) => !defined.contains(name),
        None => false,
    }
}

/// Serialized test window over the published state (`set` / `set_defined`).
///
/// The state is process-global and Rust runs tests from every module on one
/// shared thread pool, so *all* test access goes through this single lock —
/// the same reasoning as `test_support::ENV_LOCK`.  A window installs its
/// world on construction and restores the pre-driver world (fail-closed:
/// nothing published) on drop, so an assertion failure cannot leak a
/// published policy into the next test.  `backend::generation`'s remat
/// tests take a window too: the set they build must agree with the
/// inline-expansion predicate, and that predicate is fail-closed before
/// publication.
#[cfg(test)]
static POLICY_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

#[cfg(test)]
pub(crate) struct PolicyWindow {
    _guard: MutexGuard<'static, ()>,
}

#[cfg(test)]
impl Drop for PolicyWindow {
    fn drop(&mut self) {
        clear();
    }
}

/// Back to the pre-driver state: no published policy, no published
/// definitions.
#[cfg(test)]
fn clear() {
    *active_cell().write().unwrap_or_else(|e| e.into_inner()) = BuiltinPolicy::default();
    *defined_cell().write().unwrap_or_else(|e| e.into_inner()) = None;
}

#[cfg(test)]
fn policy_lock() -> MutexGuard<'static, ()> {
    POLICY_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// A compilation that has not reached codegen yet: nothing is published and
/// every expansion query fails closed.
#[cfg(test)]
pub(crate) fn unpublished_policy_for_test() -> PolicyWindow {
    let guard = policy_lock();
    clear();
    PolicyWindow { _guard: guard }
}

/// A compilation at codegen entry: the driver published `policy` and the
/// module published `defined` — the exact state `may_assume_builtin` is
/// written for.
#[cfg(test)]
pub(crate) fn published_policy_for_test(
    policy: BuiltinPolicy,
    defined: FxHashSet<String>,
) -> PolicyWindow {
    let guard = policy_lock();
    set(policy);
    set_defined(defined);
    PolicyWindow { _guard: guard }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::types::IrType;
    use crate::ir::module::IrFunction;

    #[test]
    fn unpublished_state_fails_closed() {
        let _window = unpublished_policy_for_test();
        assert!(
            !may_assume_builtin("memcpy"),
            "no module has been published: the expansion must not assume the library contract"
        );
        assert!(!may_assume_builtin("__memset_chk"));
    }

    #[test]
    fn policy_round_trips_and_per_name_withdrawal_is_exact() {
        // A published, empty definition set: everything is available.
        {
            let _window = published_policy_for_test(BuiltinPolicy::default(), FxHashSet::default());
            assert!(may_assume_builtin("memcpy"));
            assert!(may_assume_builtin("memset"));
        }

        let mut withdrawn_memcpy = BuiltinPolicy::default();
        withdrawn_memcpy.withdraw_one("memcpy");
        {
            let _window = published_policy_for_test(withdrawn_memcpy, FxHashSet::default());
            assert!(!may_assume_builtin("memcpy"), "withdrawn");
            assert!(
                may_assume_builtin("memset"),
                "per-name withdrawal is not a blanket"
            );
        }

        let mut blanket = BuiltinPolicy::default();
        blanket.withdraw_all();
        let _window = published_policy_for_test(blanket, FxHashSet::default());
        assert!(!may_assume_builtin("memcpy"));
        assert!(!may_assume_builtin("memset"));
        assert!(
            !may_assume_builtin("__memset_chk"),
            "blanket withdrawal covers the fortified spelling too"
        );
    }

    #[test]
    fn module_definitions_block_the_library_contract() {
        // Publication order: driver policy, then the module's definitions.
        let mut module = crate::ir::module::IrModule::new();
        module.functions.push(IrFunction::new(
            "memset".to_string(),
            IrType::Ptr,
            Vec::new(),
            false,
        ));
        let _window = published_policy_for_test(
            BuiltinPolicy::default(),
            crate::common::builtin::defined_symbols(&module),
        );
        assert!(
            !may_assume_builtin("memset"),
            "a TU definition blocks the expansion"
        );
        assert!(may_assume_builtin("memcpy"), "siblings unaffected");
        assert!(
            may_assume_builtin("__memset_chk"),
            "the fortified spelling is gated on the plain name, not on itself"
        );
    }
}
