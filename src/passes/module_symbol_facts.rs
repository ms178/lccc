//! Module-level symbol facts shared by transforms that reason about object identity.
//!
//! A pair of IR `GlobalAddr` roots is not enough to prove different objects:
//! GNU aliases and `asm("label")` can give one linker object multiple source
//! names, while declarations can be interposed or coalesced at link time. Only
//! distinct strong internal-linkage definitions, with no alias-like symbol
//! redirection or opaque top-level assembly, have stable disjoint identity.

use crate::common::fx_hash::FxHashSet;
use crate::ir::module::IrModule;

#[derive(Debug, Default)]
pub(crate) struct ModuleSymbolFacts {
    /// Names that participate in an alias or assembler-label redirection.
    /// Neither side of a redirection is safe as a distinct-global proof.
    pub(crate) aliased_names: FxHashSet<String>,
    /// Strong file-local definitions with stable, private object identity.
    pub(crate) local_globals: FxHashSet<String>,
}

impl ModuleSymbolFacts {
    pub(crate) fn from_module(module: &IrModule) -> Self {
        let mut aliased_names = module
            .aliases
            .iter()
            .flat_map(|(alias, target, _)| [alias.clone(), target.clone()])
            .collect::<FxHashSet<_>>();
        // An assembler label can redirect a C name to another linker symbol.
        for (name, label) in &module.asm_labels {
            aliased_names.insert(name.clone());
            aliased_names.insert(label.clone());
        }

        let mut local_globals = module
            .globals
            .iter()
            .filter(|global| {
                global.is_static && !global.is_extern && !global.is_common && !global.is_weak
            })
            .map(|global| global.name.clone())
            .collect::<FxHashSet<_>>();
        // Weak symbol attributes can be kept separately from the global's
        // declaration record; either representation must remove the proof.
        for (name, is_weak, _) in &module.symbol_attrs {
            if *is_weak {
                local_globals.remove(name);
            }
        }
        // A global name on either side of an alias/asm-label mapping is not a
        // distinct-object witness, even if its definition has internal linkage.
        local_globals.retain(|name| !aliased_names.contains(name));
        // Arbitrary assembler may define aliases the IR cannot enumerate.
        if !module.toplevel_asm.is_empty() {
            local_globals.clear();
        }

        Self {
            aliased_names,
            local_globals,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::types::IrType;
    use crate::ir::module::{GlobalInit, IrGlobal};

    fn local_global(name: &str) -> IrGlobal {
        IrGlobal {
            name: name.to_string(),
            ty: IrType::I64,
            size: 8,
            align: 8,
            init: GlobalInit::Zero,
            is_static: true,
            is_extern: false,
            is_common: false,
            section: None,
            is_weak: false,
            visibility: None,
            has_explicit_align: false,
            is_const: false,
            is_used: false,
            is_thread_local: false,
            tls_model: None,
        }
    }

    #[test]
    fn only_unredirected_strong_private_globals_keep_distinct_identity() {
        let mut module = IrModule::new();
        module.globals = ["a", "b", "c", "weak", "alias_a", "asm_b"]
            .map(local_global)
            .into();
        let mut weak_record = local_global("weak_record");
        weak_record.is_weak = true;
        let mut common = local_global("common");
        common.is_common = true;
        let mut external = local_global("external");
        external.is_extern = true;
        let mut public = local_global("public");
        public.is_static = false;
        module
            .globals
            .extend([weak_record, common, external, public]);
        module.aliases.push(("alias_a".into(), "a".into(), false));
        module.asm_labels.insert("b".into(), "asm_b".into());
        module.symbol_attrs.push(("weak".into(), true, None));

        let facts = ModuleSymbolFacts::from_module(&module);
        assert!(facts.aliased_names.contains("a"));
        assert!(facts.aliased_names.contains("alias_a"));
        assert!(facts.aliased_names.contains("b"));
        assert!(facts.aliased_names.contains("asm_b"));
        assert!(facts.local_globals.contains("c"));
        assert!(!facts.local_globals.contains("a"));
        assert!(!facts.local_globals.contains("b"));
        assert!(!facts.local_globals.contains("weak"));
        assert!(!facts.local_globals.contains("weak_record"));
        assert!(!facts.local_globals.contains("common"));
        assert!(!facts.local_globals.contains("external"));
        assert!(!facts.local_globals.contains("public"));
        assert!(!facts.local_globals.contains("alias_a"));
        assert!(!facts.local_globals.contains("asm_b"));

        module.toplevel_asm.push(".globl hidden_alias".into());
        assert!(
            ModuleSymbolFacts::from_module(&module)
                .local_globals
                .is_empty(),
            "opaque top-level assembler may redirect any global name"
        );
    }
}
