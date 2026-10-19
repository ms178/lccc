//! lccc's `const` model for `_Generic` selection.
//!
//! `CType` carries no qualifiers, so what `_Generic` needs is recorded as
//! *base constness*: whether a type's base specifier is `const` -- the type's
//! own qualifier when it is not a pointer, its innermost pointee's when it is.
//! `GenericAssociation::is_const`, `CastQualifiers::base_const`, lowering's
//! `VarInfo::base_const`, the symbol table's `const_base` mark and
//! `TypeContext::const_base_typedefs` all hold it.
//!
//! Sema and lowering each select `_Generic` associations; both use these
//! rules through [`ConstLookup`], so their selections agree.

use crate::frontend::parser::ast::{
    BinOp, Expr, GenericAssociation, PostfixOp, TypeSpecifier, UnaryOp,
};

/// The declarations a phase knows about.
pub trait ConstLookup {
    /// Base constness of the object `name` currently denotes; false for a
    /// name that denotes no object.
    fn object_const_base(&self, name: &str) -> bool;
    /// Base constness of the typedef `name`.
    fn typedef_const_base(&self, name: &str) -> bool;
    /// Whether `expr` has pointer or array type.
    fn is_pointer(&self, expr: &Expr) -> bool;
}

/// Base constness of the type named by `ts`, whose own spelling has a base
/// `const` when `explicit` is set (`const int *`), or that inherits one from a
/// typedef (`typedef const int CI; CI *`) or a `typeof` operand.
pub fn type_name_const_base(ts: &TypeSpecifier, explicit: bool, lookup: &impl ConstLookup) -> bool {
    explicit
        || match base_specifier(ts) {
            TypeSpecifier::TypedefName(name) => lookup.typedef_const_base(name),
            // `typeof` keeps every qualifier; `typeof_unqual` drops only the
            // top-level one, which is the base one unless it is a pointer.
            TypeSpecifier::Typeof(expr, unqual) => {
                (!unqual || lookup.is_pointer(expr)) && expr_const_base(expr, lookup)
            }
            TypeSpecifier::TypeofType(inner) => type_name_const_base(inner, false, lookup),
            _ => false,
        }
}

/// The specifier `ts` is derived from.
fn base_specifier(mut ts: &TypeSpecifier) -> &TypeSpecifier {
    loop {
        match ts {
            TypeSpecifier::Pointer(inner, _)
            | TypeSpecifier::Array(inner, _)
            | TypeSpecifier::FunctionPointer(inner, ..)
            | TypeSpecifier::BareFunction(inner, ..) => ts = inner,
            _ => return ts,
        }
    }
}

/// Base constness of the type of `expr`, before lvalue conversion. What no
/// declaration records (struct members, function results) reads as not
/// const.
pub fn expr_const_base(expr: &Expr, lookup: &impl ConstLookup) -> bool {
    let pointer_operand = |a: &Expr, b: &Expr| {
        let operand = if lookup.is_pointer(a) { a } else { b };
        expr_const_base(operand, lookup)
    };
    match expr {
        Expr::Identifier(name, _) => lookup.object_const_base(name),
        Expr::AddressOf(inner, _)
        | Expr::Deref(inner, _)
        | Expr::Comma(_, inner, _)
        | Expr::Assign(inner, ..)
        | Expr::CompoundAssign(_, inner, ..)
        | Expr::UnaryOp(UnaryOp::PreInc | UnaryOp::PreDec, inner, _)
        | Expr::PostfixOp(PostfixOp::PostInc | PostfixOp::PostDec, inner, _) => {
            expr_const_base(inner, lookup)
        }
        // `i[p]` is `p[i]`.
        Expr::ArraySubscript(a, b, _) => pointer_operand(a, b),
        Expr::BinaryOp(BinOp::Add | BinOp::Sub, a, b, _) => pointer_operand(a, b),
        // A composite pointer type has the qualifiers of both arms (C23
        // 6.5.15p6), so `c ? (const int *)p : q` is a `const int *`.
        Expr::Conditional(_, a, b, _) | Expr::GnuConditional(a, b, _) => {
            expr_const_base(a, lookup) || expr_const_base(b, lookup)
        }
        Expr::Cast(ts, _, _, qualifiers) => type_name_const_base(ts, qualifiers.base_const, lookup),
        _ => false,
    }
}

/// Base constness of an association's type.
pub fn association_const_base(assoc: &GenericAssociation, lookup: &impl ConstLookup) -> bool {
    assoc
        .type_spec
        .as_ref()
        .is_some_and(|ts| type_name_const_base(ts, assoc.is_const, lookup))
}

/// The constness a `_Generic` compares: the controlling operand's, and
/// whether to compare it at all.
///
/// A type operand's constness is exact (C2y), so it is always compared. An
/// expression operand undergoes lvalue conversion, which drops a non-pointer
/// operand's qualifiers and keeps a pointer's pointee ones; as an operand's
/// constness is unknown where no declaration records it, it is compared only
/// when the associations differ in constness or the operand is known const --
/// otherwise `_Generic(s.name, const char *: 1)` could not select.
pub fn controlling_const(
    controlling: &Expr,
    associations: &[GenericAssociation],
    lookup: &impl ConstLookup,
) -> (bool, bool) {
    if let Expr::TypeOperand(ts, is_const, _) = controlling {
        return (type_name_const_base(ts, *is_const, lookup), true);
    }
    let is_const = lookup.is_pointer(controlling) && expr_const_base(controlling, lookup);
    let mut typed = associations
        .iter()
        .filter(|a| a.type_spec.is_some())
        .map(|a| association_const_base(a, lookup));
    let differ = typed.clone().any(|c| c) && typed.any(|c| !c);
    (is_const, is_const || differ)
}
