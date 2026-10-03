//! The builtin-withdrawal state shared by the pass-side synthesis gate
//! (`crate::passes::libcall`) and the backend's const-size expansion gate
//! (`crate::backend::libcall_policy`).
//!
//! One object, two consumers, so the two halves of A14 can never disagree
//! about what `-fno-builtin` withdrew.  The semantics are GCC 16.2's,
//! measured on the pinned oracles (Godbolt) and re-verified locally:
//!
//! * `-fno-builtin` / `-ffreestanding` withdraw *every* name (the blanket
//!   set); `-fbuiltin` / `-fhosted` restore every name.  Last one wins.
//! * `-fno-builtin-<fn>` withdraws exactly one name and is **sticky**: a
//!   later `-fbuiltin` clears the blanket withdrawal but, as GCC does, keeps
//!   the per-name one.  (Clang diverges only for `-ffreestanding -fhosted`
//!   after a per-name withdrawal; GCC is the compatibility target.)
//!
//! A withdrawal never stops the recogniser from *seeing* a user-written
//! `memcpy(d, s, n)` — it stops optimisations from *creating* one.

use crate::common::fx_hash::FxHashSet;

/// Which library functions the CLI withdrew from builtin status.
///
/// `Default` is the no-withdrawal state the driver publishes when no
/// `-fno-builtin*` spelling was given.
#[derive(Clone, Debug, Default)]
pub(crate) struct BuiltinPolicy {
    blanket: bool,
    names: FxHashSet<String>,
}

impl BuiltinPolicy {
    /// `-fno-builtin` / `-ffreestanding`: withdraw every name.
    pub(crate) fn withdraw_all(&mut self) {
        self.blanket = true;
    }

    /// `-fbuiltin` / `-fhosted`: restore every blanket withdrawal.
    /// Per-name withdrawals survive, matching GCC.
    pub(crate) fn restore_all(&mut self) {
        self.blanket = false;
    }

    /// `-fno-builtin-<name>`: withdraw exactly one name.
    pub(crate) fn withdraw_one(&mut self, name: &str) {
        self.names.insert(name.to_string());
    }

    /// Is builtin status withdrawn for `name`?
    pub(crate) fn withdrawn(&self, name: &str) -> bool {
        self.blanket || self.names.contains(name)
    }
}

/// The linker symbols a module *defines*: every non-declaration function's
/// emitted symbol, with its `asm` label spelling when one was given, plus
/// every alias and every asm-label key.
///
/// Both gates consume this one collector, so a spelling this function
/// understands (or misses) cannot make the two disagree.  Notes on the
/// spellings, each of which is a real definition the library call could
/// resolve to:
///
/// * `void *f(...) __asm__("memcpy")` defines the symbol `memcpy`; the
///   emitted name is the label, not the C name.
/// * `void *memcpy(...) __asm__("g")` makes the C name `memcpy` denote `g`,
///   so a synthesised `memcpy` would not reach the library either — the
///   label *key* is therefore a definition of that spelling.
/// * `__attribute__((alias("target")))` defines both the alias name and the
///   target (which is why the target is inserted too).
/// * `static` counts: a TU-local name can still be the callee of the
///   synthesised call.
pub(crate) fn defined_symbols(module: &crate::ir::module::IrModule) -> FxHashSet<String> {
    let mut defined: FxHashSet<String> = FxHashSet::default();
    for func in &module.functions {
        if func.is_declaration {
            continue; // a prototype offers no body to recurse into
        }
        match module.asm_labels.get(&func.name) {
            Some(label) => {
                defined.insert(label.clone());
            }
            None => {
                defined.insert(func.name.clone());
            }
        }
    }
    for (key, label) in &module.asm_labels {
        defined.insert(key.clone());
        defined.insert(label.clone());
    }
    for (alias, target, _weak) in &module.aliases {
        defined.insert(alias.clone());
        defined.insert(target.clone());
    }
    defined
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blanket_and_per_name_withdrawals_compose() {
        let mut p = BuiltinPolicy::default();
        assert!(!p.withdrawn("memcpy"));

        p.withdraw_all();
        assert!(p.withdrawn("memcpy"));
        assert!(p.withdrawn("strlen"));

        p.restore_all();
        assert!(!p.withdrawn("memcpy"));

        p.withdraw_one("memcpy");
        assert!(p.withdrawn("memcpy"));
        assert!(
            !p.withdrawn("memmove"),
            "per-name withdrawal is not a blanket"
        );

        // -fbuiltin clears the blanket withdrawal but keeps per-name ones
        // (GCC semantics; measured on both pinned oracles).
        p.withdraw_all();
        p.restore_all();
        assert!(
            p.withdrawn("memcpy"),
            "a per-name withdrawal survives -fbuiltin"
        );
        assert!(!p.withdrawn("memmove"));
    }
}
