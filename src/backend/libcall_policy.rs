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
//!   (`generation::collect_symbol_sets`, `set_module_state`), from the same
//!   collector the pass-side gate uses (`common::builtin::symbol_inventory`).
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
//!
//! The published state is ONE value behind ONE lock.  It used to be two
//! independent globals (policy, definitions) that a second compilation in
//! the same process could interleave; a single immutable `Live` snapshot
//! makes "the policy that belongs to this symbol set" a structural
//! invariant rather than a coincidence of publication order.

use crate::common::builtin::{BuiltinPolicy, SymbolInventory};
use std::sync::OnceLock;
#[cfg(test)]
use std::sync::{Mutex, MutexGuard};

/// Everything the expansion predicate reads, published together.
#[derive(Clone, Debug, Default)]
struct Live {
    policy: BuiltinPolicy,
    inventory: SymbolInventory,
    policy_published: bool,
    inventory_published: bool,
}

static ACTIVE: OnceLock<std::sync::RwLock<Live>> = OnceLock::new();

fn cell() -> &'static std::sync::RwLock<Live> {
    ACTIVE.get_or_init(|| std::sync::RwLock::new(Live::default()))
}

fn mutate(f: impl FnOnce(&mut Live)) {
    f(&mut cell().write().unwrap_or_else(|e| e.into_inner()));
}

/// Publish the CLI's builtin-withdrawal state for this compilation.
pub(crate) fn set(policy: BuiltinPolicy) {
    mutate(|live| {
        live.policy = policy;
        // A new compilation must not inherit the previous module's inventory.
        // Keep the gate closed until codegen publishes the matching module.
        live.inventory = SymbolInventory::default();
        live.policy_published = true;
        live.inventory_published = false;
    });
}

/// Publish the module's symbol inventory, collected by
/// [`crate::common::builtin::symbol_inventory`] — the same collector the
/// pass-side gate snapshots.
pub(crate) fn set_module_state(inventory: SymbolInventory) {
    mutate(|live| {
        live.inventory = inventory;
        live.inventory_published = true;
    });
}

/// May the backend assume `name` still denotes the builtin?
///
/// Fail-closed before publication: a const-size expansion can only happen
/// once the driver has published the policy and a module has been
/// published, so an early query is a programming error, not a legitimate
/// "permissive" state — and permissive is exactly the wrong default for a
/// gate that protects against self-recursion.
pub(crate) fn may_assume_builtin(name: &str) -> bool {
    let live = cell().read().unwrap_or_else(|e| e.into_inner());
    if !live.policy_published || !live.inventory_published {
        return false;
    }
    !live.policy.withdrawn(name) && !live.inventory.blocks_library_assumption(name)
}

/// Serialized test window over the published state (`set` / `set_module_state`).
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
    mutate(|live| *live = Live::default());
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
    inventory: SymbolInventory,
) -> PolicyWindow {
    let guard = policy_lock();
    mutate(|live| {
        live.policy = policy.clone();
        live.inventory = inventory.clone();
        live.policy_published = true;
        live.inventory_published = true;
    });
    PolicyWindow { _guard: guard }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::types::IrType;
    use crate::ir::module::IrFunction;

    /// An inventory holding just these definitions.
    fn inventory_of(defined: &[&str]) -> SymbolInventory {
        SymbolInventory {
            defined: defined.iter().map(|s| s.to_string()).collect(),
            ..SymbolInventory::default()
        }
    }

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
    fn policy_alone_fails_closed_until_module_inventory_is_published() {
        let _window = unpublished_policy_for_test();
        set(BuiltinPolicy::default());
        assert!(
            !may_assume_builtin("memcpy"),
            "a fresh CLI policy without its matching module is incomplete"
        );
    }

    #[test]
    fn policy_round_trips_and_per_name_withdrawal_is_exact() {
        // A published, empty definition set: everything is available.
        {
            let _window =
                published_policy_for_test(BuiltinPolicy::default(), SymbolInventory::default());
            assert!(may_assume_builtin("memcpy"));
            assert!(may_assume_builtin("memset"));
        }

        let mut withdrawn_memcpy = BuiltinPolicy::default();
        withdrawn_memcpy.withdraw_one("memcpy");
        {
            let _window = published_policy_for_test(withdrawn_memcpy, SymbolInventory::default());
            assert!(!may_assume_builtin("memcpy"), "withdrawn");
            assert!(
                may_assume_builtin("memset"),
                "per-name withdrawal is not a blanket"
            );
        }

        let mut blanket = BuiltinPolicy::default();
        blanket.withdraw_all();
        let _window = published_policy_for_test(blanket, SymbolInventory::default());
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
            crate::common::builtin::symbol_inventory(&module),
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

    #[test]
    fn top_level_assembly_blocks_the_library_contract() {
        // The TU redefines `memcpy` in raw assembly: the compiler cannot
        // assume the library's semantics, even though no IrFunction carries
        // the name (the earlier collector missed `toplevel_asm` entirely).
        let mut module = crate::ir::module::IrModule::new();
        module
            .toplevel_asm
            .push(".globl memcpy\nmemcpy:\n\tret\n".to_string());
        let _window = published_policy_for_test(
            BuiltinPolicy::default(),
            crate::common::builtin::symbol_inventory(&module),
        );
        assert!(
            !may_assume_builtin("memcpy"),
            "assembly definition must block the expansion"
        );
        assert!(
            may_assume_builtin("memset"),
            "an unrelated symbol in the same blob is unaffected"
        );
    }

    #[test]
    fn asm_label_redirects_block_but_do_not_define() {
        // `memcpy` re-spelled to another symbol: a synthesised call through
        // the C name would land on that symbol, not on the library.
        let mut module = crate::ir::module::IrModule::new();
        let mut f = IrFunction::new("memcpy".to_string(), IrType::Ptr, Vec::new(), false);
        f.is_declaration = true; // a prototype carrying the label still redirects
        module.functions.push(f);
        module
            .asm_labels
            .insert("memcpy".to_string(), "my_copy".to_string());
        let inv = crate::common::builtin::symbol_inventory(&module);
        assert!(inv.redirected.contains("memcpy"));
        assert!(
            !inv.defined.contains("my_copy"),
            "a declaration does not define the label"
        );
        let _window = published_policy_for_test(BuiltinPolicy::default(), inv);
        assert!(!may_assume_builtin("memcpy"));
    }

    #[test]
    fn self_labelled_declaration_is_not_a_definition() {
        // glibc-style `extern ... __asm__("memcpy")`: an identity spelling.
        // Treating it as a definition used to disable the expansion in TUs
        // that never defined the callee.
        let mut module = crate::ir::module::IrModule::new();
        let mut f = IrFunction::new("memcpy".to_string(), IrType::Ptr, Vec::new(), false);
        f.is_declaration = true;
        module.functions.push(f);
        module
            .asm_labels
            .insert("memcpy".to_string(), "memcpy".to_string());
        let _window = published_policy_for_test(
            BuiltinPolicy::default(),
            crate::common::builtin::symbol_inventory(&module),
        );
        assert!(
            may_assume_builtin("memcpy"),
            "a self-label on a declaration is not a definition"
        );
    }
}
