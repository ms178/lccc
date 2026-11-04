//! GNU/C label-namespace validation (GCC 16.2 parity).
//!
//! C23 6.8.2/6.8.6.1 and the GNU labels-as-values extension require every
//! `goto name` / `&&name` / `asm goto(... : label)` to name a label DEFINED in
//! the same function. A label may be defined at most once per namespace.
//! Before this pass LCCC accepted all of the following, silently:
//!
//! * `void f(void) { goto nope; }` — the lowerer created a placeholder block
//!   that was never started, then `cfg_simplify` deleted it: `goto` fell into
//!   whichever block came next (a real miscompile).
//! * `int f(void) { void *p = &&nope; goto *p; }` — emitted `leaq .LBBn(%rip)`
//!   against an undefined local symbol; the integrated assembler rejected it,
//!   an external assembler would have bound it to absolute address 0.
//! * `void f(void) { a: ; a: ; }` — two blocks with the SAME `BlockId`, then
//!   `ccc: internal error: permutation covers every block`.
//! * `void f(void) { asm goto("" : : : : nope); }` — dangling asm-goto label.
//!
//! GCC 16.2 rejects all four (`label 'x' used but not defined` / `duplicate
//! label 'x'`, measured on this host). This pass restores that contract with
//! one linear, zero-copy AST walk per function definition. It runs BEFORE the
//! body is analysed so a bad label cannot reach lowering at all, which also
//! makes the lowering-side landing-pad backfill unambiguous: after this pass,
//! every `user_labels` entry refers to a label that IS defined in the AST.
//!
//! ## Scoping model (measured against GCC 16.2, 20 probe programs)
//!
//! * Labels are FUNCTION-scoped. A nested function cannot see an enclosing
//!   function's plain labels (`n1/n4/n8/n10` from the probe matrix: rejected).
//! * GNU `__label__` makes a label LOCAL to its block AND visible to nested
//!   functions declared inside that block (`n5/n6`: accepted; this is the
//!   documented non-local-goto mechanism used by
//!   `tests/regression/nested_nonlocal_goto_global_register.c`).
//! * A `__label__` declaration shadows an identically named outer label: the
//!   reference must be satisfied by a definition in the SAME block (`l1/l4`:
//!   rejected; `l3`, where the definition sits in a nested plain compound of
//!   the declaring block: accepted).
//! * `case`/`default` labels are a different namespace and do NOT define
//!   ordinary labels (`c2`: rejected).
//! * Undefined `&&name` in a static local initializer is an error too (`s1`).
//! * GCC reports ONE diagnostic per undefined name; this pass deduplicates.
//!
//! ## Complexity / efficiency
//!
//! One pass, O(number of AST nodes) time and O(number of labels + label
//! references) memory. Names are borrowed `&str` from the AST — no `String`
//! allocation anywhere in the walk. Frames are a flat arena indexed by `usize`
//! with an explicit parent link, so resolution is a pointer chase with no
//! recursion, no hashing of the whole AST, and no repeated subtree walks.
//! The generated machine code is untouched: this is a front-end-only check.

use crate::common::error::DiagnosticEngine;
use crate::common::fx_hash::FxHashSet;
use crate::common::source::Span;
use crate::frontend::parser::ast::{
    BlockItem, CompoundStmt, Declaration, Expr, ForInit, FunctionDef, Stmt, TypeSpecifier,
};
use std::cell::RefCell;

/// One label namespace. Frames form a tree via `parent`; the checker keeps
/// them all alive so references can be resolved in one upward walk at the end.
struct LabelScope<'a> {
    parent: Option<usize>,
    /// Labels defined (`name:`) in this frame.
    defined: FxHashSet<&'a str>,
    /// Names declared by `__label__` in this block. A reference to one of
    /// these is bound HERE: outer definitions are invisible.
    local_decls: FxHashSet<&'a str>,
    /// True for every function body (top-level AND nested). Crossing one of
    /// these frames means leaving a function's label namespace.
    function_boundary: bool,
}

impl<'a> LabelScope<'a> {
    fn function(parent: Option<usize>) -> Self {
        Self {
            parent,
            defined: FxHashSet::default(),
            local_decls: FxHashSet::default(),
            function_boundary: true,
        }
    }

    fn local_labels(parent: usize, names: &'a [String]) -> Self {
        Self {
            parent: Some(parent),
            defined: FxHashSet::default(),
            local_decls: names.iter().map(String::as_str).collect(),
            function_boundary: false,
        }
    }
}

struct LabelRef<'a> {
    name: &'a str,
    span: Span,
    /// Frame that was active when the reference was written.
    scope: usize,
}

struct LabelChecker<'a, 'd> {
    diagnostics: &'d RefCell<DiagnosticEngine>,
    scopes: Vec<LabelScope<'a>>,
    current: usize,
    refs: Vec<LabelRef<'a>>,
    /// Second (and later) definition of a name in one frame.
    duplicates: Vec<(&'a str, Span)>,
}

/// Validate the label namespace of one function definition, including every
/// nested function defined in its body. Called once per `FunctionDef`.
pub(crate) fn check_function_labels(diagnostics: &RefCell<DiagnosticEngine>, func: &FunctionDef) {
    let mut checker = LabelChecker {
        diagnostics,
        scopes: Vec::with_capacity(4),
        current: 0,
        refs: Vec::new(),
        duplicates: Vec::new(),
    };
    checker.scopes.push(LabelScope::function(None));
    checker.walk_type_spec(&func.return_type);
    for param in &func.params {
        checker.walk_param(param);
    }
    checker.walk_compound(&func.body);
    checker.report();
}

impl<'a, 'd> LabelChecker<'a, 'd> {
    #[inline]
    fn push(&mut self, scope: LabelScope<'a>) -> usize {
        let saved = self.current;
        let id = self.scopes.len();
        self.scopes.push(scope);
        self.current = id;
        saved
    }

    #[inline]
    fn pop(&mut self, saved: usize) {
        self.current = saved;
    }

    #[inline]
    fn reference(&mut self, name: &'a str, span: Span) {
        self.refs.push(LabelRef {
            name,
            span,
            scope: self.current,
        });
    }

    #[inline]
    fn define(&mut self, name: &'a str, span: Span) {
        if !self.scopes[self.current].defined.insert(name) {
            self.duplicates.push((name, span));
        }
    }

    /// Resolve one reference against the frame chain.
    ///
    /// Phase 1 walks the frames of the referencing function, innermost first,
    /// and stops at its function boundary. Phase 2 may only see `__label__`
    /// names of ENCLOSING functions — GNU's deliberate exception for nested
    /// non-local gotos — and still requires a definition in that same frame.
    fn resolves(&self, r: &LabelRef<'a>) -> bool {
        let mut at = Some(r.scope);
        // Phase 1: own function.
        while let Some(i) = at {
            let s = &self.scopes[i];
            if s.local_decls.contains(r.name) {
                return s.defined.contains(r.name);
            }
            if s.defined.contains(r.name) {
                return true;
            }
            if s.function_boundary {
                // Frames above this one belong to an enclosing function.
                at = s.parent;
                break;
            }
            at = s.parent;
        }
        // Phase 2: only __label__-exported names cross a function boundary.
        while let Some(i) = at {
            let s = &self.scopes[i];
            if s.local_decls.contains(r.name) {
                return s.defined.contains(r.name);
            }
            at = s.parent;
        }
        false
    }

    fn report(&self) {
        let mut engine = self.diagnostics.borrow_mut();
        let mut reported_undefined: FxHashSet<&'a str> = FxHashSet::default();
        for (name, span) in &self.duplicates {
            engine.error(format!("duplicate label '{}'", name), *span);
        }
        for r in &self.refs {
            if !self.resolves(r) && reported_undefined.insert(r.name) {
                engine.error(format!("label '{}' used but not defined", r.name), r.span);
            }
        }
    }

    // ── AST walk ──────────────────────────────────────────────────────────
    // Explicit matches rather than a generic visitor: `Expr::for_each_child`
    // deliberately does NOT descend into `Expr::StmtExpr` (whose compound
    // holds statements, including labelled ones), and `Stmt::for_each_own_expr`
    // does not descend into sub-statements. Both holes would let a bad label
    // through, so every variant is accounted for here.

    fn walk_compound(&mut self, c: &'a CompoundStmt) {
        let saved = if c.local_labels.is_empty() {
            None
        } else {
            Some(self.push(LabelScope::local_labels(self.current, &c.local_labels)))
        };
        for item in &c.items {
            match item {
                BlockItem::Declaration(decl) => self.walk_decl(decl),
                BlockItem::Statement(stmt) => self.walk_stmt(stmt),
                BlockItem::NestedFunction(nested) => {
                    let outer = self.push(LabelScope::function(Some(self.current)));
                    self.walk_type_spec(&nested.return_type);
                    for param in &nested.params {
                        self.walk_param(param);
                    }
                    self.walk_compound(&nested.body);
                    self.pop(outer);
                }
                BlockItem::StaticAssert(assertion) => self.walk_expr(&assertion.cond),
            }
        }
        if let Some(saved) = saved {
            self.pop(saved);
        }
    }

    fn walk_stmt(&mut self, s: &'a Stmt) {
        match s {
            Stmt::Expr(Some(e)) | Stmt::Return(Some(e), _) => self.walk_expr(e),
            Stmt::Expr(None) | Stmt::Return(None, _) => {}
            Stmt::If(cond, then_s, else_s, _) => {
                self.walk_expr(cond);
                self.walk_stmt(then_s);
                if let Some(e) = else_s {
                    self.walk_stmt(e);
                }
            }
            Stmt::While(cond, body, _) | Stmt::DoWhile(body, cond, _) => {
                self.walk_expr(cond);
                self.walk_stmt(body);
            }
            Stmt::For(init, cond, inc, body, _) => {
                match init.as_deref() {
                    Some(ForInit::Declaration(decl)) => self.walk_decl(decl),
                    Some(ForInit::Expr(e)) => self.walk_expr(e),
                    None => {}
                }
                if let Some(e) = cond {
                    self.walk_expr(e);
                }
                if let Some(e) = inc {
                    self.walk_expr(e);
                }
                self.walk_stmt(body);
            }
            Stmt::Switch(cond, body, _) => {
                self.walk_expr(cond);
                self.walk_stmt(body);
            }
            Stmt::Case(e, body, _) => {
                self.walk_expr(e);
                self.walk_stmt(body);
            }
            Stmt::CaseRange(lo, hi, body, _) => {
                self.walk_expr(lo);
                self.walk_expr(hi);
                self.walk_stmt(body);
            }
            Stmt::Default(body, _) => self.walk_stmt(body),
            Stmt::Goto(name, span) => self.reference(name.as_str(), *span),
            Stmt::GotoIndirect(e, _) => self.walk_expr(e),
            Stmt::Label(name, body, span) => {
                self.define(name.as_str(), *span);
                self.walk_stmt(body);
            }
            Stmt::Compound(c) => self.walk_compound(c),
            Stmt::Break(..) | Stmt::Continue(..) => {}
            Stmt::Declaration(decl) => self.walk_decl(decl),
            Stmt::InlineAsm {
                outputs,
                inputs,
                goto_labels,
                span,
                ..
            } => {
                for op in outputs.iter().chain(inputs) {
                    self.walk_expr(&op.expr);
                }
                for name in goto_labels {
                    self.reference(name.as_str(), *span);
                }
            }
        }
    }

    fn walk_expr(&mut self, e: &'a Expr) {
        if let Expr::LabelAddr(name, span) = e {
            self.reference(name.as_str(), *span);
        }
        // A statement expression carries statements (labels included).
        // `for_each_child` intentionally does not visit them.
        if let Expr::StmtExpr(compound, _) = e {
            self.walk_compound(compound);
        }
        e.for_each_child(&mut |child| self.walk_expr(child));
        // Type-name operands (`sizeof(type)`, casts, `_Generic` associations,
        // `__builtin_types_compatible_p`, ...) can contain `typeof(expr)`
        // whose operand is an expression. Handle them for completeness.
        e.for_each_type_name(&mut |ts| self.walk_type_spec(ts));
    }

    fn walk_type_spec(&mut self, ts: &'a TypeSpecifier) {
        ts.for_each_expr(&mut |e| self.walk_expr(e));
    }

    fn walk_param(&mut self, param: &'a crate::frontend::parser::ast::ParamDecl) {
        param.for_each_type_expr(&mut |e| self.walk_expr(e));
    }

    fn walk_decl(&mut self, d: &'a Declaration) {
        self.walk_type_spec(&d.type_spec);
        if let Some(ts) = &d.alignas_type {
            self.walk_type_spec(ts);
        }
        if let Some(ts) = &d.alignment_sizeof_type {
            self.walk_type_spec(ts);
        }
        if let Some(e) = &d.vector_size_expr {
            self.walk_expr(e);
        }
        for id in &d.declarators {
            id.for_each_expr(&mut |e| self.walk_expr(e));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checker<'a>(d: &'a RefCell<DiagnosticEngine>) -> LabelChecker<'a, 'a> {
        LabelChecker {
            diagnostics: d,
            scopes: Vec::new(),
            current: 0,
            refs: Vec::new(),
            duplicates: Vec::new(),
        }
    }

    fn ref_at(name: &str, scope: usize) -> LabelRef<'_> {
        LabelRef {
            name,
            span: Span::dummy(),
            scope,
        }
    }

    /// A plain label in the same function resolves; an undefined one does not.
    #[test]
    fn same_function_plain_label() {
        let d = RefCell::new(DiagnosticEngine::new());
        let mut c = checker(&d);
        c.scopes.push(LabelScope::function(None)); // frame 0
        assert!(!c.resolves(&ref_at("a", 0)));
        c.scopes[0].defined.insert("a");
        assert!(c.resolves(&ref_at("a", 0)));
        assert!(!c.resolves(&ref_at("b", 0)));
    }

    /// `__label__` shadows an outer definition (l1/l4 from the GCC matrix).
    #[test]
    fn local_label_declaration_shadows_outer_definition() {
        let d = RefCell::new(DiagnosticEngine::new());
        let mut c = checker(&d);
        c.scopes.push(LabelScope::function(None)); // frame 0: outer
        c.scopes[0].defined.insert("a");
        c.scopes.push(LabelScope::local_labels(0, &[])); // placeholder
        c.scopes[1].local_decls.insert("a"); // frame 1 declares, does not define
        assert!(!c.resolves(&ref_at("a", 1)), "outer a: must be invisible");
        c.scopes[1].defined.insert("a");
        assert!(c.resolves(&ref_at("a", 1)));
    }

    /// A definition in a nested plain compound of the declaring block counts.
    #[test]
    fn local_label_accepts_definition_from_nested_plain_block() {
        let d = RefCell::new(DiagnosticEngine::new());
        let mut c = checker(&d);
        c.scopes.push(LabelScope::function(None));
        c.scopes.push(LabelScope::local_labels(0, &[]));
        c.scopes[1].local_decls.insert("a");
        c.scopes[1].defined.insert("a"); // plain compound adds to the same frame
        assert!(c.resolves(&ref_at("a", 1)));
    }

    /// Plain labels are invisible across a function boundary (n1/n4/n8/n10).
    #[test]
    fn plain_labels_do_not_cross_function_boundary() {
        let d = RefCell::new(DiagnosticEngine::new());
        let mut c = checker(&d);
        c.scopes.push(LabelScope::function(None)); // frame 0: parent
        c.scopes[0].defined.insert("a");
        c.scopes.push(LabelScope::function(Some(0))); // frame 1: nested fn
        assert!(!c.resolves(&ref_at("a", 1)));
        assert!(c.resolves(&ref_at("a", 0)));
    }

    /// `__label__`-exported labels DO cross the boundary (n5/n6, nn.c).
    #[test]
    fn exported_local_labels_cross_function_boundary() {
        let d = RefCell::new(DiagnosticEngine::new());
        let mut c = checker(&d);
        c.scopes.push(LabelScope::function(None)); // frame 0: parent function
        c.scopes.push(LabelScope::local_labels(0, &[])); // frame 1: __label__ a
        c.scopes[1].local_decls.insert("a");
        c.scopes.push(LabelScope::function(Some(1))); // frame 2: nested fn
        assert!(!c.resolves(&ref_at("a", 2)), "declared but not defined");
        c.scopes[1].defined.insert("a");
        assert!(
            c.resolves(&ref_at("a", 2)),
            "defined local label is visible"
        );
    }

    /// A label defined only in a child function must not satisfy the parent.
    #[test]
    fn child_definition_is_invisible_to_parent() {
        let d = RefCell::new(DiagnosticEngine::new());
        let mut c = checker(&d);
        c.scopes.push(LabelScope::function(None)); // frame 0: parent
        c.scopes.push(LabelScope::function(Some(0))); // frame 1: nested fn
        c.scopes[1].defined.insert("b");
        assert!(!c.resolves(&ref_at("b", 0)));
        assert!(c.resolves(&ref_at("b", 1)));
    }

    /// One diagnostic per undefined name, duplicates reported once each.
    #[test]
    fn reporting_is_deduplicated() {
        let d = RefCell::new(DiagnosticEngine::new());
        let mut c = checker(&d);
        c.scopes.push(LabelScope::function(None));
        c.reference("missing", Span::dummy());
        c.reference("missing", Span::dummy());
        c.define("dup", Span::dummy());
        c.define("dup", Span::dummy());
        c.report();
        assert_eq!(d.borrow().error_count(), 2, "1 undefined + 1 duplicate");
    }
}
