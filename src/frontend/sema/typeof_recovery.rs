//! The observable half of the recovery-only `typeof` fallback contract.
//!
//! Both `typeof` resolvers (`resolve_type_spec` and
//! `resolve_type_spec_with_scope`) end in `CType::Int` when the operand's
//! type cannot be inferred. That value is a *recovery*: the translation unit
//! has already been rejected, and `int` only lets the rest of the
//! declaration be checked so one mistake yields one diagnostic instead of a
//! cascade (see the contract stated in `resolve_type_spec_with_scope`).
//!
//! A comment cannot enforce that. The failure mode it guards against is an
//! *inference gap*: an operand that sema could type in principle but that
//! `infer_expr_ctype` declines, which would silently become `int` in a
//! program that otherwise compiles. Two properties make that hard to see:
//! the fallback is silent, and the fallback is indistinguishable from a
//! genuine `typeof(int_expr)`.
//!
//! So the fallback reports itself when asked. `LCCC_TYPEOF_RECOVERY_TRACE`
//! (any non-empty value) makes every recovery print one line to stderr,
//! naming the source span of the operand. The default build pays one
//! `LazyLock` read per recovery -- recoveries only happen in translation
//! units that are already failing -- and nothing at all on a successful
//! compile.
//!
//! `tests/corpus/test_typeof_unresolved_fallback.py` uses the trace to check
//! both directions: every unresolvable shape reports a recovery, and the
//! positive matrix reports none. That turns "never the type of a program
//! that compiles" into a property the test suite observes rather than a
//! property a reader has to believe.

use std::sync::LazyLock;

use crate::frontend::parser::ast::Expr;

/// `LCCC_TYPEOF_RECOVERY_TRACE` set to anything non-empty.
static TRACE_ENABLED: LazyLock<bool> = LazyLock::new(|| {
    std::env::var_os("LCCC_TYPEOF_RECOVERY_TRACE").is_some_and(|value| !value.is_empty())
});

/// `LCCC_TYPEOF_RECOVERY_TRACE` is set: report each recovered `typeof`.
pub(crate) fn trace_enabled() -> bool {
    *TRACE_ENABLED
}

/// Report that `typeof` fell back to `CType::Int` for `expr`.
///
/// Callers must have established that inference failed; nothing here
/// decides *whether* to recover, only whether the recovery is visible.
pub(crate) fn record_fallback(expr: &Expr) {
    if !trace_enabled() {
        return;
    }
    // `{:?}` rather than a pretty-printer: the trace exists to name the
    // operand for a human debugging a mistyped declaration, and the AST
    // debug form is exact, always available, and panic-free.
    crate::cli::stderr_line(format_args!(
        "typeof-recovery: CType::Int for operand {expr:?}"
    ));
}
