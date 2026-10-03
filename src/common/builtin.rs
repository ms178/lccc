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

/// What a module does to a symbol name, split by *why* the compiler must
/// not assume the library's semantics for it.
///
/// The reasons need different handling, which is why they are separate sets
/// rather than one bag of "names":
///
/// * [`SymbolInventory::defined`] — this TU answers the call itself: a
///   function or object definition under its emitted name, an alias
///   (`__attribute__((alias))` defines both the alias name and its target),
///   or a top-level `asm("...")` blob that names the symbol (the blob's
///   syntax is not fully modelled — a mention is treated as a definition, the
///   conservative reading).
/// * [`SymbolInventory::objects`] — the source declares the name as an object,
///   including an `extern` declaration. A newly synthesized function call must
///   not bind to a symbol the translation unit identifies as data.
/// * [`SymbolInventory::redirected`] — the name does not reach the library
///   even though this TU may not define it: `__asm__("label")` re-spells the
///   emitted symbol, so `void *memcpy(...) __asm__("g")` makes every
///   `memcpy` call in this TU emit a call to `g` — a synthesised `memcpy`
///   call would land in the same place.  A **declaration** carrying an `asm`
///   label does the same for callers, which is why declarations count here.
///   A label equal to the C name is an identity, not a redirect, and must
///   not suppress anything (glibc headers contain such spellings; treating
///   them as definitions used to kill the synthesis optimisations in TUs
///   that never defined the callee).
///
/// `static` counts: a TU-local name can still be the callee of the
/// synthesised call.
#[derive(Clone, Debug, Default)]
pub(crate) struct SymbolInventory {
    pub(crate) defined: FxHashSet<String>,
    pub(crate) objects: FxHashSet<String>,
    pub(crate) redirected: FxHashSet<String>,
    /// Top-level `asm("...")` blobs, verbatim.  Kept as text because the
    /// question "does this blob define `X`?" is only ever asked for a
    /// handful of candidate names (see [`crate::common::asm_scan`]), and a
    /// mention-based answer cannot miss a spelling the way a parser can.
    pub(crate) asm_blobs: Vec<String>,
}

impl SymbolInventory {
    /// Must the compiler refuse to assume that `name` still denotes the
    /// library function of that name?
    pub(crate) fn blocks_library_assumption(&self, name: &str) -> bool {
        self.defined.contains(name)
            || self.objects.contains(name)
            || self.redirected.contains(name)
            || crate::common::asm_scan::any_blob_mentions(&self.asm_blobs, name)
    }
}

/// Collect every symbol relationship the gates must respect from `module`.
///
/// Both gates consume this one collector, so a spelling this function
/// understands (or misses) cannot make the two disagree.
pub(crate) fn symbol_inventory(module: &crate::ir::module::IrModule) -> SymbolInventory {
    let mut inv = SymbolInventory {
        asm_blobs: module.toplevel_asm.clone(),
        ..SymbolInventory::default()
    };

    // `asm_labels` is keyed by the source-level C name, while definitions in
    // the lowered module already carry their emitted label. Record redirects
    // directly from the map; looking it up by `func.name` misses definitions
    // such as `memcpy __asm__("user_copy")`, whose IrFunction is named
    // `user_copy` by the time this collector runs.
    for (name, label) in &module.asm_labels {
        if label != name {
            inv.redirected.insert(name.clone());
        }
    }

    // Definitions are stored under emitted names (including asm labels).
    for func in &module.functions {
        if !func.is_declaration {
            inv.defined.insert(func.name.clone());
        }
    }
    for global in &module.globals {
        // Keep object declarations too: even an extern object named `memcpy`
        // is evidence that this identifier denotes data, not a library
        // function that an optimisation may introduce a call to.
        inv.objects.insert(global.name.clone());
        if !global.is_extern {
            inv.defined.insert(global.name.clone());
        }
    }
    for (alias, target, _weak) in &module.aliases {
        inv.defined.insert(alias.clone());
        inv.defined.insert(target.clone());
    }
    inv
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::module::IrModule;

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

    // ---- symbol inventory (A13/A14) --------------------------------------
    //
    // One collector feeds both gates, so its semantics are pinned here.  Each
    // of these shapes was a review finding: the collector missed top-level
    // assembly entirely, and it treated a declaration (which may still bind to
    // the library at link time) as a definition (which cannot).

    fn function(name: &str, is_declaration: bool) -> crate::ir::module::IrFunction {
        let mut f = crate::ir::module::IrFunction::new(
            name.to_string(),
            crate::common::types::IrType::I32,
            Vec::new(),
            false,
        );
        f.is_declaration = is_declaration;
        f
    }

    fn global(name: &str, is_extern: bool) -> crate::ir::module::IrGlobal {
        crate::ir::module::IrGlobal {
            name: name.to_string(),
            ty: crate::common::types::IrType::I32,
            size: 4,
            align: 4,
            init: crate::ir::module::GlobalInit::Zero,
            is_static: false,
            is_extern,
            is_common: false,
            section: None,
            is_weak: false,
            visibility: None,
            has_explicit_align: false,
            is_const: false,
            is_used: false,
            is_thread_local: false,
        }
    }

    #[test]
    fn a_definition_blocks_the_library_assumption() {
        let mut m = IrModule::new();
        m.functions.push(function("memcpy", false));
        let inv = symbol_inventory(&m);
        assert!(inv.blocks_library_assumption("memcpy"));
        assert!(!inv.blocks_library_assumption("memset"), "per-symbol");
    }

    #[test]
    fn a_declaration_does_not() {
        // `void *memcpy(void *, const void *, size_t);` says the symbol is
        // referenced, not provided: the call still reaches the library, so a
        // synthesised call is exactly what the user asked for.  Treating this
        // as a definition would silently disable every libcall rewrite in a
        // TU that merely declares the function it later calls.
        let mut m = IrModule::new();
        m.functions.push(function("memcpy", true));
        let inv = symbol_inventory(&m);
        assert!(!inv.blocks_library_assumption("memcpy"));
        assert!(!inv.defined.contains("memcpy"));
    }

    #[test]
    fn object_symbols_block_function_synthesis() {
        let mut defined = IrModule::new();
        defined.globals.push(global("memcpy", false));
        let inv = symbol_inventory(&defined);
        assert!(inv.blocks_library_assumption("memcpy"));
        assert!(!inv.blocks_library_assumption("memset"));

        // Even an extern object declaration establishes that this identifier
        // denotes data rather than a library function.
        let mut declared = IrModule::new();
        declared.globals.push(global("memcpy", true));
        assert!(symbol_inventory(&declared).blocks_library_assumption("memcpy"));
    }

    #[test]
    fn an_asm_label_redirect_blocks_both_spellings() {
        // Lowered definitions are stored under the emitted label, while the
        // map remains keyed by the source-level C name. Both the redirect and
        // the local body therefore need to block a synthesized call.
        for (source_name, emitted_name) in [("f", "memcpy"), ("memcpy", "g")] {
            let mut m = IrModule::new();
            m.functions.push(function(emitted_name, false));
            m.asm_labels
                .insert(source_name.to_string(), emitted_name.to_string());
            let inv = symbol_inventory(&m);
            assert!(inv.blocks_library_assumption(source_name));
            assert!(inv.blocks_library_assumption(emitted_name));
        }
    }

    #[test]
    fn an_asm_label_on_a_declaration_is_a_redirect_only() {
        // A declaration with an asm label promises nothing: the emitted
        // reference uses the label, so the C name no longer denotes the
        // library function, but nothing is defined here.
        let mut m = IrModule::new();
        m.functions.push(function("printf", true));
        m.asm_labels
            .insert("printf".to_string(), "my_printf".to_string());
        let inv = symbol_inventory(&m);
        assert!(
            inv.blocks_library_assumption("printf"),
            "the name redirects"
        );
        assert!(
            !inv.blocks_library_assumption("my_printf"),
            "a declaration defines nothing"
        );
    }

    #[test]
    fn a_self_label_is_neither_a_redirect_nor_a_missing_definition() {
        // `asm("memcpy")` is a no-op redirect: the name is still the name, and
        // a definition of it is still a definition.
        let mut m = IrModule::new();
        m.functions.push(function("memcpy", false));
        m.asm_labels
            .insert("memcpy".to_string(), "memcpy".to_string());
        let inv = symbol_inventory(&m);
        assert!(inv.blocks_library_assumption("memcpy"));
        assert!(inv.defined.contains("memcpy"));
        assert!(inv.redirected.is_empty());
    }

    #[test]
    fn toplevel_assembly_is_scanned_for_the_symbol() {
        // The review finding: `asm(".globl memset\nmemset: ...")` defines the
        // symbol without a C-level definition, so the collector must keep the
        // blobs and ask the shared scanner.
        let mut m = IrModule::new();
        m.toplevel_asm
            .push(".globl memset\nmemset:\n  ret\n".to_string());
        let inv = symbol_inventory(&m);
        assert!(inv.blocks_library_assumption("memset"));
        assert!(
            !inv.blocks_library_assumption("memcpy"),
            "whole identifiers only"
        );
        // A mention that never defines anything still blocks: the bundle has
        // no way to prove the blob does not define the symbol, and the cost of
        // being wrong is a self-call.
        let mut m = IrModule::new();
        m.toplevel_asm.push("call memmove@PLT".to_string());
        assert!(symbol_inventory(&m).blocks_library_assumption("memmove"));
    }

    #[test]
    fn an_alias_defines_both_names() {
        let mut m = IrModule::new();
        m.aliases
            .push(("my_memcpy".to_string(), "memcpy".to_string(), false));
        let inv = symbol_inventory(&m);
        assert!(inv.blocks_library_assumption("my_memcpy"));
        assert!(inv.blocks_library_assumption("memcpy"));
    }
}
