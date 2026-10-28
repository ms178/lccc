//! Module-level symbol facts shared by transforms that reason about object identity.
//!
//! A pair of IR `GlobalAddr` roots is not enough to prove different objects:
//! GNU aliases and `asm("label")` can give one linker object multiple source
//! names, while two public declarations can be interposed or coalesced at link
//! time.
//!
//! The identity witness is GCC's `symtab_node::equal_address_to` rule without
//! its alias-oracle shortcut (which assumes *any* two variables differ): a
//! name is a witness when this unit holds its strong (not weak, not common)
//! definition, that definition binds locally, and no alias or assembler label
//! here redirects it.  Such a name cannot share storage with a differently
//! named object:
//!
//! * GNU C requires an alias to be defined in the same unit as its target, so
//!   every alias of a definition made here is visible in `aliased_names`;
//! * the only remaining way another name could reach this storage is symbol
//!   interposition, which a locally binding definition excludes (`static`,
//!   hidden/internal/protected visibility, or any definition in executable
//!   code — see [`crate::common::tls::binds_locally`]).
//!
//! A declaration (`extern`) is never a witness: another unit may define it as
//! an alias of something else (`tests/regression/check_loop_idiom.sh` runs
//! exactly that cross-unit case).  Two names are distinct objects when neither
//! is redirected and at least one is a witness ([`ModuleSymbolFacts::globals_distinct`]),
//! the single answer every memory transform asks.

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::ir::module::IrModule;

#[derive(Debug, Default)]
pub(crate) struct ModuleSymbolFacts {
    /// Names that participate in an alias or assembler-label redirection.
    /// Neither side of a redirection is safe as a distinct-global proof.
    pub(crate) aliased_names: FxHashSet<String>,
    /// Strong, unredirected, locally binding definitions in this unit: object
    /// identities no other name can reach (see the module documentation).
    pub(crate) identity_witnesses: FxHashSet<String>,
    /// Minimum allocated byte size for strong, locally binding definitions in
    /// `identity_witnesses`. Interposable, weak, common, or aliased symbols are
    /// excluded so bounds proofs never assume a replaceable definition's size.
    pub(crate) defined_sizes: FxHashMap<String, usize>,
}

impl ModuleSymbolFacts {
    /// Whether two global roots name provably different objects.
    pub(crate) fn globals_distinct(&self, a: &str, b: &str) -> bool {
        a != b
            && (self.identity_witnesses.contains(a) || self.identity_witnesses.contains(b))
            && !self.aliased_names.contains(a)
            && !self.aliased_names.contains(b)
    }

    /// Proven minimum allocated byte size of a locally binding global object.
    pub(crate) fn global_min_bytes(&self, name: &str) -> Option<usize> {
        self.defined_sizes.get(name).copied()
    }

    /// `shlib` is GCC's `flag_shlib` (`-fPIC`/`-fpic`/`-shared`): only there
    /// can a public definition be interposed.
    pub(crate) fn from_module(module: &IrModule, shlib: bool) -> Self {
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

        // Redeclarations are merged per name the way `tls::model_table` merges
        // them (defined if any record defines it, `static` if any record is,
        // first explicit visibility wins); a weak or common mark on *any*
        // record, or in a separate attribute record, removes the witness.
        #[derive(Default)]
        struct Decl<'a> {
            is_static: bool,
            defined: bool,
            weak_or_common: bool,
            visibility: Option<&'a str>,
            defined_size: usize,
        }
        let mut decls: FxHashMap<&str, Decl<'_>> = FxHashMap::default();
        for g in &module.globals {
            let d = decls.entry(g.name.as_str()).or_default();
            d.is_static |= g.is_static;
            if !g.is_extern {
                d.defined = true;
                d.defined_size = d.defined_size.max(g.size);
            }
            d.weak_or_common |= g.is_weak || g.is_common;
            if d.visibility.is_none() {
                d.visibility = g.visibility.as_deref();
            }
        }
        for (name, is_weak, vis) in &module.symbol_attrs {
            if let Some(d) = decls.get_mut(name.as_str()) {
                d.weak_or_common |= *is_weak;
                if d.visibility.is_none() {
                    d.visibility = vis.as_deref();
                }
            }
        }
        // Arbitrary assembler may define aliases the IR cannot enumerate.
        let mut defined_sizes = FxHashMap::default();
        let identity_witnesses = if module.toplevel_asm.is_empty() {
            decls
                .into_iter()
                .filter(|(name, d)| {
                    d.defined
                        && !d.weak_or_common
                        && !aliased_names.contains(*name)
                        && crate::common::tls::binds_locally(shlib, d.is_static, true, d.visibility)
                })
                .map(|(name, d)| {
                    if d.defined_size > 0 {
                        defined_sizes.insert(name.to_string(), d.defined_size);
                    }
                    name.to_string()
                })
                .collect()
        } else {
            FxHashSet::default()
        };

        Self {
            aliased_names,
            identity_witnesses,
            defined_sizes,
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
            is_retain: false,
            is_nocommon: false,
        }
    }

    fn witnesses(module: &IrModule, shlib: bool) -> Vec<String> {
        let mut v: Vec<String> = ModuleSymbolFacts::from_module(module, shlib)
            .identity_witnesses
            .into_iter()
            .collect();
        v.sort();
        v
    }

    #[test]
    fn only_unredirected_strong_local_definitions_are_identity_witnesses() {
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
        let mut hidden = local_global("hidden");
        hidden.is_static = false;
        hidden.visibility = Some("hidden".into());
        let mut hidden_decl = local_global("hidden_decl");
        hidden_decl.is_static = false;
        hidden_decl.is_extern = true;
        hidden_decl.visibility = Some("hidden".into());
        module
            .globals
            .extend([weak_record, common, external, public, hidden, hidden_decl]);
        module.aliases.push(("alias_a".into(), "a".into(), false));
        module.asm_labels.insert("b".into(), "asm_b".into());
        module.symbol_attrs.push(("weak".into(), true, None));

        let facts = ModuleSymbolFacts::from_module(&module, false);
        for redirected in ["a", "alias_a", "b", "asm_b"] {
            assert!(facts.aliased_names.contains(redirected), "{redirected}");
        }
        // Executable code: every strong definition binds to itself, public or
        // not; declarations, weak/common definitions and redirected names
        // never witness.
        assert_eq!(witnesses(&module, false), ["c", "hidden", "public"]);
        // Shared-library code: a default-visibility public definition can be
        // interposed, so only `static` and hidden definitions remain.  A hidden
        // *declaration* binds locally but may be another unit's alias.
        assert_eq!(witnesses(&module, true), ["c", "hidden"]);

        module.toplevel_asm.push(".globl hidden_alias".into());
        assert!(
            witnesses(&module, false).is_empty(),
            "opaque top-level assembler may redirect any global name"
        );
    }

    #[test]
    fn a_weak_or_common_redeclaration_removes_the_witness() {
        // `int x; extern int x __attribute__((weak));` and the common form
        // arrive as separate records of one name: any record's mark counts.
        let mut module = IrModule::new();
        let mut weak_redecl = local_global("x");
        weak_redecl.is_extern = true;
        weak_redecl.is_weak = true;
        let mut common_redecl = local_global("y");
        common_redecl.is_common = true;
        module.globals = vec![
            local_global("x"),
            weak_redecl,
            common_redecl,
            local_global("y"),
        ];
        assert!(witnesses(&module, false).is_empty());
    }

    #[test]
    fn redeclarations_merge_definition_static_and_visibility() {
        // `extern int p; int p = 1;` is a definition; `extern int h
        // __attribute__((visibility("hidden"))); int h;` binds locally in a
        // shared library through the first explicit visibility.
        let mut module = IrModule::new();
        let mut p_decl = local_global("p");
        p_decl.is_static = false;
        p_decl.is_extern = true;
        let mut p_def = local_global("p");
        p_def.is_static = false;
        let mut h_decl = local_global("h");
        h_decl.is_static = false;
        h_decl.is_extern = true;
        h_decl.visibility = Some("hidden".into());
        let mut h_def = local_global("h");
        h_def.is_static = false;
        module.globals = vec![p_decl, p_def, h_decl, h_def];
        assert_eq!(witnesses(&module, false), ["h", "p"]);
        assert_eq!(witnesses(&module, true), ["h"]);
        // A visibility written only in an attribute record counts as well.
        module.globals.retain(|g| g.name == "p");
        module
            .symbol_attrs
            .push(("p".into(), false, Some("hidden".into())));
        assert_eq!(witnesses(&module, true), ["p"]);
        // Protected *data* does not bind locally in shared-library code: the
        // executable may copy-relocate it (GCC's `extern_protected_data` on
        // x86, `tls::binds_locally`), so it is not a witness there.
        module.symbol_attrs.clear();
        module.globals[0].visibility = Some("protected".into());
        assert_eq!(witnesses(&module, false), ["p"]);
        assert!(witnesses(&module, true).is_empty());
    }

    #[test]
    fn distinct_needs_one_witness_and_no_redirection() {
        let facts = ModuleSymbolFacts {
            aliased_names: ["w_alias", "w2"].map(str::to_owned).into_iter().collect(),
            identity_witnesses: ["w", "w2"].map(str::to_owned).into_iter().collect(),
            ..ModuleSymbolFacts::default()
        };
        assert!(facts.globals_distinct("w", "decl"));
        assert!(facts.globals_distinct("decl", "w"));
        assert!(!facts.globals_distinct("w", "w"));
        // Two declarations: another unit may define one as the other's alias.
        assert!(!facts.globals_distinct("decl_a", "decl_b"));
        // The witness's own alias reaches its storage.
        assert!(!facts.globals_distinct("w", "w_alias"));
        // A redirected name never witnesses, whatever the set says.
        assert!(!facts.globals_distinct("w2", "decl"));
    }
}
