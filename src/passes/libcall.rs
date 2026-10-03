//! Libcall synthesis policy — which library calls an optimisation pass may
//! *introduce* into a translation unit.
//!
//! Several passes replace user code with a call to a standard library
//! function (`loop_idiom` → `memcpy`/`memmove`, `loop_memset` → `memset`,
//! `fortify_fold` → `puts`/`fputs`/`putchar`/`fputc`/`fwrite`).  That is a
//! pure win whenever the callee is the real library function, but it is
//! **unsound** when the symbol could resolve to a definition the synthesised
//! call is itself part of.  Three reproduced cases (A13):
//!
//! 1. `void *memcpy(...) { for (...) d[i] = s[i]; }` — `loop_idiom` rewrote
//!    the loop inside `memcpy` into `call memmove@PLT` (at `-O2`/`-O3`).
//!    On libcs whose `memmove` calls `memcpy` that is unbounded
//!    self-recursion.
//! 2. `static void helper(...) { for (...) ... } void *memcpy(...) {
//!    helper(d, s, n); }` — inlining merges `helper` into `memcpy` *before*
//!    the pass runs, so renaming the loop's function does not help; the
//!    TU's definition of the symbol does.
//! 3. `void *memset(...) { for (...) p[i] = 0; }` — the same self-call for
//!    `loop_memset`.
//!
//! The rule implemented here is the one every reference compiler already
//! follows (measured with the pinned oracles, GCC 16.2 / Clang 23.1 /
//! ICX 2025, on all three shapes): **a pass may introduce a call to `f` only
//! when this translation unit does not define `f`.**  The check is per
//! *symbol*, not per enclosing function.
//!
//! The check is closed over *interposable* calls: a synthesised `memmove`
//! may be implemented by calling the public `memcpy` (newlib and the
//! historical BSD/glibc `memmove` forward the non-overlapping case that
//! way), so a TU that defines `memcpy` blocks `memmove` synthesis too —
//! which is what all three reference compilers do (measured: with a
//! TU-defined `memcpy`, GCC 16.2, Clang 23.1 and ICX 2025 all keep every
//! copy loop scalar, 9/9 compiler × flag rows).  The reverse edge is not
//! assumed: this image's glibc reaches its memcpy through the private
//! `__memcpy_*` entry (`audit/reach2.c`, hits = 0), and no measured
//! implementation of `memcpy`/`memset` calls the public `memmove`.
//!
//! Withdrawn names keep the ordinary calls the user wrote — the gates stop
//! *synthesis* only.  The loop itself is left intact and still reaches the
//! vectoriser, so `-fno-builtin` costs coverage of one idiom, not
//! performance.

use crate::common::fx_hash::FxHashSet;
use crate::ir::instruction::Instruction;
use crate::ir::module::IrModule;

/// The CLI's builtin-withdrawal state.  Defined in `crate::common::builtin`
/// because the backend consults the same policy when it expands calls (A14);
/// re-exported here so the pass-side call sites keep one import path.
pub(crate) use crate::common::builtin::BuiltinPolicy;

/// Symbols that a library implementation of `name` may call *through an
/// interposable symbol*, so that a synthesised `name` call can reach them.
///
/// Only evidence-backed edges belong here: an extra entry costs a rewrite in
/// TUs that define the reached symbol, a missing one costs correctness.  See
/// the module docs for the measurement behind the direction.
fn libcall_reach(name: &str) -> &'static [&'static str] {
    match name {
        // newlib / BSD / historical glibc `memmove` call the public `memcpy`
        // for the non-overlapping case.  The edge is defensive: this image's
        // glibc does not (private `__memcpy_*` entry), but the reference
        // compilers refuse the rewrite under a TU-defined `memcpy`, and that
        // is the compatibility target.
        "memmove" => &["memcpy"],
        _ => &[],
    }
}

/// The stdio functions `fortify_fold` can synthesise a call to.
pub(crate) const STDIO_FOLD_TARGETS: &[&str] = &["puts", "fputs", "putchar", "fputc", "fwrite"];

/// The symbols a pass may synthesise calls to, for one compilation.
///
/// # Contract
///
/// [`may_use`](Self::may_use) is the only gate a pass may consult before it
/// emits a call the user did not write.  For `name` it returns `false` when
///
/// * this TU defines `name` — the definition may be the function being
///   compiled, or (after inlining) part of it, or a non-standard replacement
///   the user is entitled to;
/// * `name` appears as a key in `asm_labels` — the TU stated that this C
///   name denotes some other linker symbol, so a synthesised reference
///   through it would not reach the library function either;
/// * a CLI knob withdrew builtin status for it ([`BuiltinPolicy`]);
/// * a symbol the callee's library implementation may call is defined here
///   ([`libcall_reach`]) — the call would then reach the user's definition
///   transitively.
#[derive(Clone, Debug)]
pub(crate) struct LibcallAllowance {
    /// Linker symbols this TU defines (functions, plus alias and asm-label
    /// spellings of them).
    defined: FxHashSet<String>,
    /// CLI withdrawals.
    policy: BuiltinPolicy,
}

impl LibcallAllowance {
    /// Snapshot the definitions of `module` and the CLI withdrawals.
    ///
    /// Called once per compilation, before the pass pipeline runs.  Later
    /// passes do not add definitions of library functions (inlining merges
    /// bodies, it does not introduce new symbols), so a single snapshot is
    /// stable for the whole pipeline.  A pass that removes a now-dead
    /// definition can only make this answer more conservative.
    pub(crate) fn from_module(module: &IrModule, policy: BuiltinPolicy) -> Self {
        // Collection lives in `common::builtin` because the backend needs
        // the identical set for its expansion policy (A14b).
        Self {
            defined: crate::common::builtin::defined_symbols(module),
            policy,
        }
    }

    /// May a pass emit a call to library function `name`?
    pub(crate) fn may_use(&self, name: &str) -> bool {
        if self.defined.contains(name) || self.policy.withdrawn(name) {
            return false;
        }
        // Transitive edge: the implementation may call these through the
        // linker, so a definition here is reachable from the new call.
        !libcall_reach(name)
            .iter()
            .any(|reached| self.defined.contains(*reached))
    }

    /// An allowance that permits everything, for unit tests of pass
    /// mechanics that predate the policy (and for tests that assert the
    /// recogniser is unchanged when the policy is permissive).
    #[cfg(test)]
    pub(crate) fn unrestricted() -> Self {
        Self {
            defined: FxHashSet::default(),
            policy: BuiltinPolicy::default(),
        }
    }
}

/// May this `fortify_fold` decision be applied?  Scans the replacement
/// instructions for a call to any stdio synthesis target and requires
/// [`LibcallAllowance::may_use`] for each one found.  Decisions that only
/// drop a call, or whose replacement contains no synthesised call, are
/// always permitted.
pub(crate) fn stdio_fold_permitted(insts: &[Instruction], libcalls: &LibcallAllowance) -> bool {
    for inst in insts {
        if let Instruction::Call { func, .. } = inst {
            if STDIO_FOLD_TARGETS.contains(&func.as_str()) && !libcalls.may_use(func) {
                return false;
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::types::IrType;
    use crate::ir::module::IrFunction;

    fn module_with_defined(names: &[&str]) -> IrModule {
        let mut m = IrModule::new();
        for name in names {
            // `IrFunction::new` starts from a definition (`is_declaration`
            // false), which is the state under test.
            m.functions.push(IrFunction::new(
                name.to_string(),
                IrType::I32,
                Vec::new(),
                false,
            ));
        }
        m
    }

    #[test]
    fn pristine_tu_may_synthesise() {
        let a =
            LibcallAllowance::from_module(&module_with_defined(&["usercode"]), Default::default());
        for name in ["memcpy", "memmove", "memset", "strlen", "puts"] {
            assert!(a.may_use(name), "{name} must stay available");
        }
    }

    #[test]
    fn declaration_of_a_libcall_does_not_block_it() {
        // A *declaration* is not a definition: the library function is still
        // the callee.
        let mut m = IrModule::new();
        let mut f = IrFunction::new("memcpy".to_string(), IrType::Ptr, Vec::new(), false);
        f.is_declaration = true;
        m.functions.push(f);
        let a = LibcallAllowance::from_module(&m, Default::default());
        assert!(
            a.may_use("memcpy"),
            "a declaration offers no body to recurse into"
        );
    }

    #[test]
    fn tu_definition_blocks_exactly_that_symbol() {
        // "Exactly" means per symbol, never per family: defining one of the
        // `mem*` functions leaves every sibling available.
        let a =
            LibcallAllowance::from_module(&module_with_defined(&["memset"]), Default::default());
        assert!(!a.may_use("memset"), "defined here: recursion risk");
        assert!(a.may_use("memcpy"), "a sibling is unaffected");
        assert!(a.may_use("memmove"));
        assert!(a.may_use("strlen"));
        assert!(a.may_use("puts"));

        // The single deliberate exception is the `memmove` -> `memcpy` reach
        // edge, asserted here so this test cannot silently drift from the
        // closure contract; the full closure is covered by
        // `memmove_synthesis_is_closed_over_memcpy_definitions`.
        let b =
            LibcallAllowance::from_module(&module_with_defined(&["memcpy"]), Default::default());
        assert!(!b.may_use("memcpy"));
        assert!(
            !b.may_use("memmove"),
            "libc memmove may call the public memcpy defined here"
        );
        assert!(
            b.may_use("memset"),
            "the closure is one edge, not the whole family"
        );
    }

    #[test]
    fn asm_label_remap_blocks_both_spellings() {
        // `void *f(...) __asm__("memcpy");` defines symbol `memcpy` while the
        // C name is `f`; `void *memcpy(...) __asm__("g");` declares `memcpy`
        // as a name for symbol `g`.  Both must block a synthesised `memcpy`.
        for (defined_fn, label) in [("f", "memcpy"), ("memcpy", "g")] {
            let mut m = module_with_defined(&[defined_fn]);
            m.asm_labels
                .insert(defined_fn.to_string(), label.to_string());
            let a = LibcallAllowance::from_module(&m, Default::default());
            assert!(
                !a.may_use("memcpy"),
                "asm label {label} for {defined_fn} must block memcpy synthesis"
            );
        }
    }

    #[test]
    fn alias_target_blocks_synthesis() {
        let mut m = module_with_defined(&["usercode"]);
        m.aliases
            .push(("memcpy_alias".to_string(), "memcpy".to_string(), false));
        let a = LibcallAllowance::from_module(&m, Default::default());
        assert!(!a.may_use("memcpy"));
        assert!(!a.may_use("memcpy_alias"));
    }

    #[test]
    fn memmove_synthesis_is_closed_over_memcpy_definitions() {
        // A TU-defined memcpy blocks memmove too: the library's memmove may
        // call the public memcpy, and all three reference compilers keep
        // copy loops scalar in that TU.
        let a =
            LibcallAllowance::from_module(&module_with_defined(&["memcpy"]), Default::default());
        assert!(!a.may_use("memcpy"));
        assert!(!a.may_use("memmove"), "memmove reaches the TU's memcpy");
        assert!(
            a.may_use("memset"),
            "the edge is per-symbol, not family-wide"
        );
        // The reverse direction is deliberately not assumed.
        let b =
            LibcallAllowance::from_module(&module_with_defined(&["memmove"]), Default::default());
        assert!(!b.may_use("memmove"));
        assert!(
            b.may_use("memcpy"),
            "no measured memcpy implementation calls public memmove"
        );
    }

    #[test]
    fn blanket_withdrawal_blocks_everything_until_restored() {
        let mut p = BuiltinPolicy::default();
        p.withdraw_all();
        let a = LibcallAllowance::from_module(&module_with_defined(&["x"]), p.clone());
        assert!(!a.may_use("memcpy"));
        assert!(!a.may_use("memset"));
        p.restore_all();
        let a = LibcallAllowance::from_module(&module_with_defined(&["x"]), p);
        assert!(a.may_use("memcpy"), "-fhosted restores the blanket set");
    }

    #[test]
    fn per_name_withdrawal_is_exact_and_survives_restore() {
        let mut p = BuiltinPolicy::default();
        p.withdraw_one("memcpy");
        let a = LibcallAllowance::from_module(&module_with_defined(&["x"]), p.clone());
        assert!(!a.may_use("memcpy"));
        assert!(a.may_use("memmove"), "per-name withdrawal is not a blanket");
        assert!(a.may_use("memset"));
        p.restore_all();
        let a = LibcallAllowance::from_module(&module_with_defined(&["x"]), p);
        assert!(
            !a.may_use("memcpy"),
            "-fbuiltin clears the blanket withdrawal but keeps per-name ones (GCC)"
        );
        assert!(a.may_use("memmove"));
    }

    #[test]
    fn stdio_fold_is_permitted_only_for_allowed_targets() {
        use crate::ir::instruction::CallInfo;
        use crate::ir::reexports::Value;

        fn puts_call(name: &str) -> Instruction {
            Instruction::Call {
                func: name.to_string(),
                info: CallInfo {
                    dest: None,
                    args: Vec::new(),
                    arg_types: Vec::new(),
                    return_type: IrType::I32,
                    is_variadic: false,
                    num_fixed_args: 0,
                    struct_arg_sizes: Vec::new(),
                    struct_arg_aligns: Vec::new(),
                    struct_arg_classes: Vec::new(),
                    struct_arg_riscv_float_classes: Vec::new(),
                    struct_arg_is_f128_sse: Vec::new(),
                    ret_is_f128_sse: false,
                    is_sret: false,
                    is_fastcall: false,
                    regparm: None,
                    is_pure: false,
                    is_const: false,
                    ret_eightbyte_classes: Vec::new(),
                },
            }
        }
        let _ = Value(0);

        let open = LibcallAllowance::unrestricted();
        assert!(stdio_fold_permitted(&[puts_call("puts")], &open));

        let mut p = BuiltinPolicy::default();
        p.withdraw_one("puts");
        let closed = LibcallAllowance::from_module(&module_with_defined(&["x"]), p);
        assert!(!stdio_fold_permitted(&[puts_call("puts")], &closed));
        assert!(
            stdio_fold_permitted(&[puts_call("putchar")], &closed),
            "the withdrawal is per-name"
        );

        // A TU definition of the produced symbol blocks it too.
        let defined =
            LibcallAllowance::from_module(&module_with_defined(&["fputs"]), Default::default());
        assert!(!stdio_fold_permitted(&[puts_call("fputs")], &defined));
        assert!(
            stdio_fold_permitted(&[puts_call("puts")], &defined),
            "sibling target stays available"
        );

        // A decision with no call at all (pure call removal) is always fine.
        assert!(stdio_fold_permitted(&[], &closed));
    }
}
