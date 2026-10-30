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
    BinOp, BlockItem, Expr, GenericAssociation, PostfixOp, Stmt, TypeSpecifier, UnaryOp,
};

/// The declarations a phase knows about.
pub trait ConstLookup {
    /// Base constness of the object `name` currently denotes; false for a
    /// name that denotes no object.
    fn object_const_base(&self, name: &str) -> bool;
    /// Base constness of the typedef `name`.
    fn typedef_const_base(&self, name: &str) -> bool;
    /// The pointer level qualified by `const name`, if `name` denotes a
    /// pointer or an array of pointers. Levels are zero-based, from the base.
    fn typedef_const_pointer_level(&self, name: &str) -> Option<usize>;
    /// Whether `expr` has pointer or array type.
    fn is_pointer(&self, expr: &Expr) -> bool;
    /// Pointer level that a `const` on this expression's type would qualify.
    /// Arrays are transparent, so an array of pointers has a level but an
    /// array of integers does not.
    fn expr_const_pointer_level(&self, expr: &Expr) -> Option<usize>;
    /// The same level for a type name, including typedef and typeof operands.
    fn type_const_pointer_level(&self, ts: &TypeSpecifier) -> Option<usize>;
    /// Resolve the selected association expression of a `_Generic` selection.
    fn select_generic<'a>(
        &self,
        _controlling: &Expr,
        _associations: &'a [GenericAssociation],
    ) -> Option<&'a Expr> {
        None
    }
    /// Recorded base constness of member `field` on `base` (`base->field` when
    /// `indirect` is true, `base.field` otherwise).
    fn member_const_base(&self, _base: &Expr, _field: &str, _indirect: bool) -> bool {
        false
    }
    /// Recorded base constness of the return type of a call to `callee`, when
    /// `callee` resolves to a known function signature.
    fn call_const_base(&self, _callee: &Expr) -> Option<bool> {
        None
    }
}

/// Base constness of the type named by `ts`, whose own spelling has a base
/// `const` when `explicit` is set (`const int *`), or that inherits one from a
/// typedef (`typedef const int CI; CI *`) or a `typeof` operand.
pub fn type_name_const_base(ts: &TypeSpecifier, explicit: bool, lookup: &impl ConstLookup) -> bool {
    match base_specifier(ts) {
        // Qualifying a pointer typedef qualifies the pointer, not its target:
        // `typedef int *P; const P p` is `int *const p`. A typedef can already
        // carry a const pointee independently (`typedef const int *P`).
        TypeSpecifier::TypedefName(name) => {
            lookup.typedef_const_base(name)
                || (explicit && lookup.typedef_const_pointer_level(name).is_none())
        }
        // `const typeof(pointer)` qualifies that pointer, just like a pointer
        // typedef, not its pointee. For `typeof_unqual`, only the type's own
        // qualifier is removed; a pointer's pointee remains unchanged. Arrays
        // of pointers qualify their elements, while arrays of scalars retain
        // the scalar-base rule.
        TypeSpecifier::Typeof(expr, unqual) => {
            let pointer = lookup.expr_const_pointer_level(expr).is_some();
            (explicit && !pointer) || ((!unqual || pointer) && expr_const_base(expr, lookup))
        }
        TypeSpecifier::TypeofType(inner, quals) => {
            let pointer = lookup.type_const_pointer_level(inner).is_some();
            (explicit && !pointer)
                || ((!quals.unqual || pointer)
                    && type_name_const_base(inner, quals.base_const, lookup))
        }
        _ => explicit,
    }
}

/// For a typedef, return the zero-based pointer level qualified by `const T`.
/// Arrays are transparent to qualification: `const A` for `typedef int *A[2]`
/// has const pointer elements. Count inner pointers too so that the outermost
/// pointer's index is measured from the innermost base, as in the CVR vectors.
pub fn typedef_const_pointer_level(ty: &crate::common::types::CType) -> Option<usize> {
    use crate::common::types::CType;
    let mut ty = ty;
    while let CType::Array(inner, _) = ty {
        ty = inner;
    }
    if !matches!(ty, CType::Pointer(..)) {
        return None;
    }
    let mut pointers = 0;
    loop {
        match ty {
            CType::Pointer(inner, _) => {
                pointers += 1;
                ty = inner;
            }
            CType::Array(inner, _) => ty = inner,
            _ => return Some(pointers - 1),
        }
    }
}

/// The specifier `ts` is derived from.
pub fn base_specifier(mut ts: &TypeSpecifier) -> &TypeSpecifier {
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
        Expr::CompoundLiteral(ts, _, _, st) => {
            type_name_const_base(ts, st.base_const || st.is_constexpr, lookup)
        }
        Expr::GenericSelection(controlling, associations, _) => lookup
            .select_generic(controlling, associations)
            .is_some_and(|selected| expr_const_base(selected, lookup)),
        Expr::PointerMemberAccess(base, field, _) => {
            lookup.member_const_base(base, field, true)
                || (!lookup.is_pointer(expr) && expr_const_base(base, lookup))
        }
        Expr::MemberAccess(base, field, _) => {
            lookup.member_const_base(base, field, false)
                || (!lookup.is_pointer(expr)
                    && !lookup.is_pointer(base)
                    && expr_const_base(base, lookup))
        }
        Expr::StmtExpr(compound, _) => match compound.syntactic_last_item() {
            Some(BlockItem::Statement(Stmt::Expr(Some(last)))) => expr_const_base(last, lookup),
            _ => false,
        },
        Expr::FunctionCall(callee, _, _) => lookup.call_const_base(callee).unwrap_or(false),
        _ => false,
    }
}

/// Whether `expr`'s base constness is recorded by declarations rather than
/// falling back to the unrecorded default.
fn expr_const_recorded(expr: &Expr, lookup: &impl ConstLookup) -> bool {
    let pointer_operand = |a: &Expr, b: &Expr| {
        let operand = if lookup.is_pointer(a) { a } else { b };
        expr_const_recorded(operand, lookup)
    };
    match expr {
        Expr::Identifier(..)
        | Expr::Cast(..)
        | Expr::CompoundLiteral(..)
        | Expr::StringLiteral(..)
        | Expr::WideStringLiteral(..)
        | Expr::Char16StringLiteral(..)
        | Expr::PointerMemberAccess(..)
        | Expr::MemberAccess(..) => true,
        Expr::AddressOf(inner, _)
        | Expr::Deref(inner, _)
        | Expr::Comma(_, inner, _)
        | Expr::Assign(inner, ..)
        | Expr::CompoundAssign(_, inner, ..)
        | Expr::UnaryOp(UnaryOp::PreInc | UnaryOp::PreDec, inner, _)
        | Expr::PostfixOp(PostfixOp::PostInc | PostfixOp::PostDec, inner, _) => {
            expr_const_recorded(inner, lookup)
        }
        Expr::ArraySubscript(a, b, _) | Expr::BinaryOp(BinOp::Add | BinOp::Sub, a, b, _) => {
            pointer_operand(a, b)
        }
        Expr::Conditional(_, a, b, _) | Expr::GnuConditional(a, b, _) => {
            expr_const_recorded(a, lookup) && expr_const_recorded(b, lookup)
        }
        Expr::GenericSelection(controlling, associations, _) => lookup
            .select_generic(controlling, associations)
            .is_some_and(|selected| expr_const_recorded(selected, lookup)),
        Expr::StmtExpr(compound, _) => match compound.syntactic_last_item() {
            Some(BlockItem::Statement(Stmt::Expr(Some(last)))) => expr_const_recorded(last, lookup),
            _ => false,
        },
        Expr::FunctionCall(callee, _, _) => lookup.call_const_base(callee).is_some(),
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
/// operand's qualifiers and keeps a pointer's pointee ones; when a pointer
/// operand's constness is recorded and `default:` is present (such as Linux's
/// `container_of_const`), constness is also compared so a non-const pointer
/// selects `default:` rather than a `const T *` association.
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
    let has_default = associations.iter().any(|a| a.type_spec.is_none());
    let known_pointer = lookup.is_pointer(controlling) && expr_const_recorded(controlling, lookup);
    (
        is_const,
        is_const || differ || (has_default && known_pointer),
    )
}
