//! Dead static function elimination via reference collection.
//!
//! This module implements dead static function elimination by walking AST compound
//! statements to collect all referenced function names. Functions that are declared
//! as static but never referenced (transitively) from any non-static function body
//! or global initializer can be safely skipped during lowering, providing a significant
//! performance win for translation units that include many header-defined static
//! or inline functions.

use super::lower::Lowerer;
use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::frontend::parser::ast::{
    BlockItem, CompoundStmt, Declaration, Expr, ExternalDecl, ForInit, FunctionDef, Initializer,
    Stmt, TranslationUnit,
};
use std::collections::VecDeque;

impl Lowerer {
    /// Returns true if a function definition can be skipped when unreferenced.
    /// This mirrors the skip logic in `lower()` — static, static inline,
    /// extern inline with gnu_inline attribute or in GNU89 mode, and C99
    /// inline-only definitions can all be skipped.
    fn is_skippable_function(&self, func: &FunctionDef) -> bool {
        let is_gnu_inline_no_extern_def = self.is_gnu_inline_no_extern_def(&func.attrs);
        // C99 plain `inline` (without `extern` or `static`) does not provide
        // an external definition. These are lowered as static when referenced,
        // so they can be skipped when unreferenced.
        // Per C99 6.7.4p7: unless there's a non-inline declaration of this function.
        // Note: in GNU89 mode, `inline` without `extern` provides an external def,
        // so this rule does not apply.
        let is_c99_inline_only = !self.gnu89_inline
            && func.attrs.is_inline()
            && !func.attrs.is_extern()
            && !func.attrs.is_static()
            && !func.attrs.is_gnu_inline()
            && !self.has_non_inline_decl.contains(&func.name);
        func.attrs.is_static() || is_gnu_inline_no_extern_def || is_c99_inline_only
    }

    /// Collect all function names that are transitively referenced from root
    /// (non-skippable) function bodies and global initializers.
    ///
    /// Algorithm:
    /// 1. Collect direct references from root functions and global initializers
    /// 2. Build a per-function reference map for skippable functions
    /// 3. Worklist-based transitive closure: when a skippable function becomes
    ///    reachable, add its references to the worklist
    pub(super) fn collect_referenced_static_functions(
        &self,
        tu: &TranslationUnit,
    ) -> FxHashSet<String> {
        let mut referenced = FxHashSet::default();
        // Map from skippable function name -> set of functions it references
        let mut skippable_refs: FxHashMap<String, FxHashSet<String>> = FxHashMap::default();

        for decl in &tu.decls {
            match decl {
                ExternalDecl::FunctionDef(func) => {
                    if self.is_skippable_function(func) && !func.attrs.is_used() {
                        // For skippable functions, collect their refs into a separate map
                        // so we can do transitive closure later
                        let mut func_refs = FxHashSet::default();
                        self.collect_refs_from_function(func, &mut func_refs);
                        skippable_refs.insert(func.name.clone(), func_refs);
                    } else {
                        // Root function: collect refs directly into the referenced set.
                        // __attribute__((used)) functions (e.g. glibc's `static
                        // __attribute__((__used__)) _dl_start`, called from asm)
                        // are roots even though they are static — otherwise their
                        // callees (rtld_timer_start etc.) get skipped and the
                        // link fails with undefined references.
                        if func.attrs.is_used() {
                            referenced.insert(func.name.clone());
                        }
                        self.collect_refs_from_function(func, &mut referenced);
                    }
                }
                ExternalDecl::Declaration(decl) => {
                    self.collect_refs_from_declaration(decl, &mut referenced);
                    for declarator in &decl.declarators {
                        // Alias targets reference the aliased function
                        if let Some(ref target) = declarator.attrs.alias_target {
                            referenced.insert(target.clone());
                        }
                    }
                }
                ExternalDecl::TopLevelAsm(_) | ExternalDecl::StaticAssert(_) => {
                    // Neither references C functions.
                }
            }
        }

        // Constructor and destructor functions are roots: they will be called by
        // the runtime via .init_array/.fini_array, so any functions they reference
        // must also be preserved. The constructor/destructor names were collected
        // into module.constructors/destructors in an earlier pass.
        for ctor in &self.module.constructors {
            referenced.insert(ctor.clone());
        }
        for dtor in &self.module.destructors {
            referenced.insert(dtor.clone());
        }

        // Transitive closure: use a worklist to propagate reachability through
        // skippable functions. When a skippable function is found to be referenced,
        // add all of its own references to the worklist.
        let mut worklist: VecDeque<String> = referenced.iter().cloned().collect();
        while let Some(name) = worklist.pop_front() {
            if let Some(func_refs) = skippable_refs.get(&name) {
                for r in func_refs {
                    if referenced.insert(r.clone()) {
                        worklist.push_back(r.clone());
                    }
                }
            }
        }

        referenced
    }

    /// Collect the functions a function definition references: in its
    /// parameter types (a variably modified parameter's bounds are evaluated
    /// on entry) and in its body.
    fn collect_refs_from_function(&self, func: &FunctionDef, refs: &mut FxHashSet<String>) {
        func.return_type
            .for_each_expr(&mut |e| self.collect_refs_from_expr(e, refs));
        for param in &func.params {
            param.for_each_type_expr(&mut |e| self.collect_refs_from_expr(e, refs));
        }
        self.collect_refs_from_compound(&func.body, refs);
    }

    /// Collect function name references from a compound statement.
    pub(super) fn collect_refs_from_compound(
        &self,
        compound: &CompoundStmt,
        refs: &mut FxHashSet<String>,
    ) {
        for item in &compound.items {
            match item {
                BlockItem::Declaration(decl) => self.collect_refs_from_declaration(decl, refs),
                BlockItem::Statement(stmt) => self.collect_refs_from_stmt(stmt, refs),
                // Decided by sema; a static assertion generates nothing.
                BlockItem::StaticAssert(_) => {}
                // A nested function is reachable only from its parent, so
                // its references keep callees alive like the parent's own.
                BlockItem::NestedFunction(nested) => self.collect_refs_from_function(nested, refs),
            }
        }
    }

    /// Collect the functions a declaration references: in its type (VLA
    /// bounds, `typeof`), its declarators' array bounds and parameter types,
    /// its initializers, and `__attribute__((cleanup(fn)))`.
    pub(super) fn collect_refs_from_declaration(
        &self,
        decl: &Declaration,
        refs: &mut FxHashSet<String>,
    ) {
        let mut visit = |e: &Expr| self.collect_refs_from_expr(e, refs);
        decl.type_spec.for_each_expr(&mut visit);
        if let Some(ts) = &decl.alignas_type {
            ts.for_each_expr(&mut visit);
        }
        for declarator in &decl.declarators {
            for derived in &declarator.derived {
                derived.for_each_expr(&mut visit);
            }
            if let Some(init) = &declarator.init {
                init.for_each_expr(&mut visit);
            }
        }
        for declarator in &decl.declarators {
            if let Some(cleanup_fn) = &declarator.attrs.cleanup_fn {
                if self.known_functions.contains(cleanup_fn) {
                    refs.insert(cleanup_fn.clone());
                }
            }
        }
    }

    /// Collect function name references from a statement and the statements
    /// nested in it.
    pub(super) fn collect_refs_from_stmt(&self, stmt: &Stmt, refs: &mut FxHashSet<String>) {
        stmt.for_each_own_expr(&mut |e| self.collect_refs_from_expr(e, refs));
        match stmt {
            Stmt::Compound(compound) => self.collect_refs_from_compound(compound, refs),
            Stmt::If(_, then_s, else_s, _) => {
                self.collect_refs_from_stmt(then_s, refs);
                if let Some(else_s) = else_s {
                    self.collect_refs_from_stmt(else_s, refs);
                }
            }
            Stmt::For(init, _, _, body, _) => {
                if let Some(init) = init {
                    if let ForInit::Declaration(decl) = init.as_ref() {
                        self.collect_refs_from_declaration(decl, refs);
                    }
                }
                self.collect_refs_from_stmt(body, refs);
            }
            Stmt::While(_, body, _)
            | Stmt::DoWhile(body, ..)
            | Stmt::Switch(_, body, _)
            | Stmt::Case(_, body, _)
            | Stmt::CaseRange(_, _, body, _)
            | Stmt::Default(body, _)
            | Stmt::Label(_, body, _) => self.collect_refs_from_stmt(body, refs),
            Stmt::Declaration(decl) => self.collect_refs_from_declaration(decl, refs),
            Stmt::Expr(_)
            | Stmt::Return(..)
            | Stmt::Break(_)
            | Stmt::Continue(_)
            | Stmt::Goto(..)
            | Stmt::GotoIndirect(..)
            | Stmt::InlineAsm { .. } => {}
        }
    }

    /// Collect the functions an expression may reference when lowered.
    ///
    /// Deliberately a superset: every sub-expression and every expression in
    /// the type-names it is written with (VLA bounds, `typeof`) counts,
    /// including unevaluated `sizeof`/`_Alignof`/`typeof` operands -- whether
    /// one is evaluated depends on its type being variably modified, which
    /// lowering decides. A static function kept only by such a reference is
    /// lowered and then removed by `dead_statics`, which works on the IR's
    /// actual references and runs at every optimization level; missing a
    /// reference instead leaves an undefined symbol at link time.
    pub(super) fn collect_refs_from_expr(&self, expr: &Expr, refs: &mut FxHashSet<String>) {
        let mut pending = vec![expr];
        while let Some(e) = pending.pop() {
            match e {
                Expr::Identifier(name, _) => {
                    if self.known_functions.contains(name) {
                        refs.insert(name.clone());
                    }
                }
                Expr::StmtExpr(compound, _) => self.collect_refs_from_compound(compound, refs),
                _ => {}
            }
            e.for_each_type_name(&mut |ts| ts.for_each_expr(&mut |bound| pending.push(bound)));
            e.for_each_child(&mut |child| pending.push(child));
        }
    }
}
