//! Operand-type constraints of C23 6.5 and 6.8.4-6.8.5 for the operand kinds
//! that have no valid use as arithmetic or truth values: structures and
//! unions, `void`, and C23 `nullptr_t`.
//!
//! lccc used to accept these operands and hand them to lowering, which has no
//! meaning for them: `a + b` on two (struct-based) `__m128i` values compiled
//! into a binary that segfaulted, where GCC 16.2 and Clang 23.1.3 reject or
//! define the program. Each rule below is a constraint both oracles enforce
//! as a hard error, with the messages GCC uses.
//!
//! The rules are pure functions of the operand types (already decayed, see
//! [`CType::decayed_value_ctype`]) and of whether an operand is a null pointer
//! constant, so they are unit-tested here without building expressions. The
//! analyzer decides which inferred types are exact enough to check (see
//! `SemanticAnalyzer::constraint_operand`); every other combination, such as
//! GCC's pointer/integer leniency, is outside this module and unchanged.

use crate::common::types::CType;
use crate::frontend::parser::ast::{BinOp, UnaryOp};

/// The operand classes these constraints distinguish.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Record,
    Void,
    NullPtr,
    Pointer,
    /// GNU and ext vectors. A record against a vector is left alone: lccc's
    /// `__m128`-family types are structures and builtin results are not
    /// typed exactly enough here to judge those pairings.
    Vector,
    Other,
}

fn kind(ty: &CType) -> Kind {
    match ty {
        CType::Struct(_) | CType::Union(_) => Kind::Record,
        CType::Void => Kind::Void,
        CType::NullPtr => Kind::NullPtr,
        CType::Pointer(..) | CType::Array(..) | CType::Function(_) => Kind::Pointer,
        CType::Vector(..) | CType::ExtVector(..) => Kind::Vector,
        _ => Kind::Other,
    }
}

/// Whether a constraint here can concern an operand of this type. Callers use
/// it to skip the null-pointer-constant evaluation for ordinary operands.
pub(super) fn involved(ty: &CType) -> bool {
    matches!(kind(ty), Kind::Record | Kind::Void | Kind::NullPtr)
}

/// A record paired with a vector, which these rules do not judge.
fn record_vector_pair(a: Kind, b: Kind) -> bool {
    matches!(
        (a, b),
        (Kind::Record, Kind::Vector) | (Kind::Vector, Kind::Record)
    )
}

/// One checked operand: its value type and whether it is a null pointer
/// constant (an integer constant expression 0, one cast to `void *`, or
/// `nullptr`; C23 6.3.2.3p3).
#[derive(Clone, Copy, Debug)]
pub(super) struct Operand<'a> {
    pub ty: &'a CType,
    pub null_constant: bool,
}

const VOID_VALUE: &str = "void value not ignored as it ought to be";

/// GCC's "used struct type value where scalar is required", for a controlling
/// expression, `!`, `&&`, `||` or the first operand of `?:`. `nullptr_t` is a
/// scalar type and allowed (it converts to `false`).
pub(super) fn scalar_required(ty: &CType) -> Option<String> {
    match ty {
        CType::Struct(_) => Some("used struct type value where scalar is required".into()),
        CType::Union(_) => Some("used union type value where scalar is required".into()),
        CType::Void => Some(VOID_VALUE.into()),
        _ => None,
    }
}

/// An operand whose value is used where no conversion target constrains it,
/// such as a variadic argument: only a `void` operand is invalid.
pub(super) fn value_required(ty: &CType) -> Option<String> {
    matches!(ty, CType::Void).then(|| VOID_VALUE.into())
}

/// Unary `+`, `-`, `~`, `!` and `__real__`/`__imag__`.
pub(super) fn unary(op: UnaryOp, ty: &CType) -> Option<String> {
    let what = match op {
        UnaryOp::Plus => "unary plus",
        UnaryOp::Neg => "unary minus",
        UnaryOp::BitNot => "bit-complement",
        UnaryOp::LogicalNot => "unary exclamation mark",
        UnaryOp::PreInc => "increment",
        UnaryOp::PreDec => "decrement",
        // GCC: `__real__`/`__imag__` of a record or `nullptr_t` is "wrong
        // type argument to __real__"; of `void`, the void-value error.
        UnaryOp::RealPart => "__real__",
        UnaryOp::ImagPart => "__imag__",
    };
    match kind(ty) {
        Kind::Void => Some(VOID_VALUE.into()),
        Kind::Record => Some(format!("wrong type argument to {what}")),
        // `!nullptr` is valid (a scalar operand); every arithmetic unary
        // operator requires an arithmetic operand (C23 6.5.3.3p1).
        Kind::NullPtr if op != UnaryOp::LogicalNot => {
            Some(format!("wrong type argument to {what}"))
        }
        _ => None,
    }
}

/// Postfix and prefix `++`/`--` (C23 6.5.2.4p1, 6.5.3.1p1): a real or pointer
/// operand.
pub(super) fn increment(increment: bool, ty: &CType) -> Option<String> {
    let what = if increment { "increment" } else { "decrement" };
    match kind(ty) {
        Kind::Void => Some(VOID_VALUE.into()),
        Kind::Record | Kind::NullPtr => Some(format!("wrong type argument to {what}")),
        _ => None,
    }
}

fn binop_spelling(op: BinOp) -> &'static str {
    match op {
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
        BinOp::Mod => "%",
        BinOp::BitAnd => "&",
        BinOp::BitOr => "|",
        BinOp::BitXor => "^",
        BinOp::Shl => "<<",
        BinOp::Shr => ">>",
        BinOp::Eq => "==",
        BinOp::Ne => "!=",
        BinOp::Lt => "<",
        BinOp::Le => "<=",
        BinOp::Gt => ">",
        BinOp::Ge => ">=",
        BinOp::LogicalAnd => "&&",
        BinOp::LogicalOr => "||",
    }
}

/// Binary operators. `&&`/`||` take any scalar operands. `==`/`!=` accept a
/// `nullptr_t` operand against another `nullptr_t`, a pointer, or a null
/// pointer constant (C23 6.5.9p2); every other operator rejects it, and every
/// operator rejects a record or `void` operand.
pub(super) fn binary(op: BinOp, lhs: Operand<'_>, rhs: Operand<'_>) -> Option<String> {
    let (lk, rk) = (kind(lhs.ty), kind(rhs.ty));
    if lk == Kind::Void || rk == Kind::Void {
        return Some(VOID_VALUE.into());
    }
    if matches!(op, BinOp::LogicalAnd | BinOp::LogicalOr) {
        return scalar_required(lhs.ty).or_else(|| scalar_required(rhs.ty));
    }
    let nullptr_ok = |other: Operand<'_>| {
        matches!(kind(other.ty), Kind::NullPtr | Kind::Pointer) || other.null_constant
    };
    let valid = match (lk, rk) {
        _ if record_vector_pair(lk, rk) => true,
        (Kind::Record, _) | (_, Kind::Record) => false,
        (Kind::NullPtr, _) | (_, Kind::NullPtr) if matches!(op, BinOp::Eq | BinOp::Ne) => {
            (lk != Kind::NullPtr || nullptr_ok(rhs)) && (rk != Kind::NullPtr || nullptr_ok(lhs))
        }
        (Kind::NullPtr, _) | (_, Kind::NullPtr) => false,
        _ => true,
    };
    (!valid).then(|| {
        format!(
            "invalid operands to binary {} (have '{}' and '{}')",
            binop_spelling(op),
            lhs.ty,
            rhs.ty
        )
    })
}

/// The second and third operands of `?:` (C23 6.5.15p3). A `nullptr_t` arm
/// pairs with another `nullptr_t` or a pointer (a null pointer constant of
/// integer type is not enough: both oracles reject `c ? 0 : nullptr`), and a
/// record arm pairs only with a record. One `void` arm against a non-void one
/// is GCC's accepted extension and is left alone.
pub(super) fn conditional_arms(then_ty: &CType, else_ty: &CType) -> Option<String> {
    let (tk, ek) = (kind(then_ty), kind(else_ty));
    let valid = match (tk, ek) {
        _ if record_vector_pair(tk, ek) => true,
        (Kind::Void, _) | (_, Kind::Void) => true,
        (Kind::Record, Kind::Record) => true,
        (Kind::Record, _) | (_, Kind::Record) => false,
        (Kind::NullPtr, other) | (other, Kind::NullPtr) => {
            matches!(other, Kind::NullPtr | Kind::Pointer)
        }
        _ => true,
    };
    (!valid).then(|| "type mismatch in conditional expression".to_string())
}

/// Simple assignment, and the conversions "as if by assignment" of
/// initialization, argument passing and `return` (C23 6.5.16.1p1), for a
/// `nullptr_t`, `void` or record operand on either side. Record-to-record
/// compatibility stays with the callers, which know the anonymous-record key
/// caveats.
pub(super) fn assignment(from: Operand<'_>, to: &CType) -> Option<String> {
    let (fk, tk) = (kind(from.ty), kind(to));
    let valid = match (fk, tk) {
        (Kind::Void, _) => return Some(VOID_VALUE.into()),
        // An object of type `void` cannot exist; nothing to diagnose here.
        (_, Kind::Void) => true,
        _ if record_vector_pair(fk, tk) => true,
        (Kind::Record, Kind::Record) => true,
        (Kind::Record, _) | (_, Kind::Record) => false,
        (_, Kind::NullPtr) => fk == Kind::NullPtr || from.null_constant,
        (Kind::NullPtr, _) => tk == Kind::Pointer || matches!(to, CType::Bool),
        _ => true,
    };
    (!valid).then(|| {
        format!(
            "incompatible types (have '{}' but expected '{}')",
            from.ty, to
        )
    })
}

/// Explicit casts (C23 6.5.4p2-p4, plus GCC's cast to a union from one of its
/// member types). `union_has_member` answers whether the target union has a
/// member of the operand's type.
pub(super) fn cast(
    from: Operand<'_>,
    to: &CType,
    union_has_member: impl FnOnce() -> bool,
) -> Option<String> {
    let fk = kind(from.ty);
    match to {
        CType::Void => return None,
        CType::Struct(_) => {
            return (from.ty != to).then(|| "conversion to non-scalar type requested".into());
        }
        CType::Union(_) => {
            return (from.ty != to && !union_has_member())
                .then(|| "cast to union type from type not present in union".into());
        }
        _ => {}
    }
    match fk {
        Kind::Void => return Some("invalid use of void expression".into()),
        // GCC's equal-size record/vector reinterpretation is not judged here.
        Kind::Record if kind(to) == Kind::Vector => return None,
        Kind::Record => {
            let expected = if to.is_integer() {
                "an integer"
            } else if to.is_floating() || to.is_complex() {
                "a floating-point"
            } else if kind(to) == Kind::Pointer {
                "a pointer"
            } else {
                "a scalar"
            };
            return Some(format!(
                "aggregate value used where {expected} was expected"
            ));
        }
        _ => {}
    }
    let valid = match (fk, kind(to)) {
        // Only `nullptr_t` itself or a null pointer constant converts to it.
        (_, Kind::NullPtr) => fk == Kind::NullPtr || from.null_constant,
        // ... and it converts only to `void`, `bool` and pointer types.
        (Kind::NullPtr, tk) => tk == Kind::Pointer || matches!(to, CType::Bool),
        _ => true,
    };
    (!valid).then(|| format!("conversion from '{}' to '{}'", from.ty, to))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::types::AddressSpace;

    fn op(ty: &CType) -> Operand<'_> {
        Operand {
            ty,
            null_constant: false,
        }
    }
    fn npc(ty: &CType) -> Operand<'_> {
        Operand {
            ty,
            null_constant: true,
        }
    }
    fn ptr(to: CType) -> CType {
        CType::Pointer(Box::new(to), AddressSpace::Default)
    }

    /// The constraint matrix measured on GCC 16.2 and Clang 23.1.3, which
    /// agree on every case (`-std=c23`, one statement per translation unit).
    #[test]
    fn nullptr_t_constraints_match_the_oracles() {
        let (n, int, dbl, bool_) = (CType::NullPtr, CType::Int, CType::Double, CType::Bool);
        let p = ptr(CType::Int);
        let vp = ptr(CType::Void);
        // Initialization / assignment.
        assert!(assignment(op(&n), &int).is_some(), "int a = nullptr");
        assert!(assignment(op(&n), &dbl).is_some(), "double a = nullptr");
        assert!(assignment(op(&p), &n).is_some(), "nullptr_t m = p");
        assert!(assignment(op(&n), &p).is_none(), "int *q = n");
        assert!(assignment(op(&n), &bool_).is_none(), "bool b = n");
        assert!(assignment(npc(&int), &n).is_none(), "nullptr_t m = 0");
        assert!(assignment(op(&int), &n).is_some(), "nullptr_t m = x");
        // Casts.
        assert!(cast(op(&n), &int, || false).is_some(), "(int)nullptr");
        assert!(cast(op(&n), &CType::Float, || false).is_some(), "(float)n");
        assert!(cast(op(&n), &bool_, || false).is_none(), "(bool)n");
        assert!(cast(op(&n), &p, || false).is_none(), "(int *)n");
        assert!(cast(op(&n), &CType::Void, || false).is_none(), "(void)n");
        assert!(cast(op(&p), &n, || false).is_some(), "(nullptr_t)p");
        assert!(cast(op(&int), &n, || false).is_some(), "(nullptr_t)1");
        assert!(cast(npc(&int), &n, || false).is_none(), "(nullptr_t)0");
        // Operators.
        assert!(
            binary(BinOp::Add, op(&n), op(&int)).is_some(),
            "nullptr + 1"
        );
        assert!(binary(BinOp::Eq, op(&n), op(&int)).is_some(), "n == 1");
        assert!(binary(BinOp::Eq, op(&n), npc(&int)).is_none(), "n == 0");
        assert!(binary(BinOp::Eq, npc(&int), op(&n)).is_none(), "0 == n");
        assert!(
            binary(BinOp::Eq, op(&n), npc(&vp)).is_none(),
            "n == (void *)0"
        );
        assert!(binary(BinOp::Ne, op(&n), op(&p)).is_none(), "n != p");
        assert!(binary(BinOp::Eq, op(&n), op(&n)).is_none(), "n == n");
        assert!(binary(BinOp::Lt, op(&n), op(&p)).is_some(), "n < p");
        assert!(binary(BinOp::Lt, op(&n), op(&n)).is_some(), "n < nullptr");
        assert!(
            binary(BinOp::LogicalAnd, op(&n), op(&int)).is_none(),
            "n && x"
        );
        assert!(unary(UnaryOp::Neg, &n).is_some(), "-n");
        assert!(unary(UnaryOp::LogicalNot, &n).is_none(), "!n");
        assert!(increment(true, &n).is_some(), "n++");
        assert!(scalar_required(&n).is_none(), "if (n)");
        // Conditional operator.
        assert!(conditional_arms(&int, &n).is_some(), "x ? 0 : nullptr");
        assert!(conditional_arms(&dbl, &n).is_some(), "x ? 1.0 : n");
        assert!(conditional_arms(&n, &p).is_none(), "x ? nullptr : p");
        assert!(
            conditional_arms(&n, &vp).is_none(),
            "x ? nullptr : (void *)0"
        );
        assert!(conditional_arms(&n, &n).is_none(), "x ? n : n");
    }

    /// Record and `void` operands: every case is a GCC 16.2 hard error (also
    /// under `-fpermissive`) that lccc used to accept.
    #[test]
    fn record_and_void_operands_are_rejected() {
        let s = CType::Struct("struct.S".into());
        let u = CType::Union("union.U".into());
        let (int, void, p) = (CType::Int, CType::Void, ptr(CType::Int));
        assert!(assignment(op(&s), &int).is_some(), "int a = s");
        assert!(assignment(op(&int), &s).is_some(), "s = 1");
        assert!(
            assignment(op(&s), &s).is_none(),
            "record pairs are the caller's"
        );
        assert!(assignment(op(&void), &int).is_some(), "int a = v()");
        assert!(
            cast(op(&s), &int, || false).unwrap().contains("an integer"),
            "(int)s"
        );
        assert!(
            cast(op(&s), &p, || false).unwrap().contains("a pointer"),
            "(int *)s"
        );
        assert!(
            cast(op(&s), &CType::Double, || false)
                .unwrap()
                .contains("floating")
        );
        assert!(cast(op(&s), &s, || false).is_none(), "(struct S)s");
        assert!(cast(op(&int), &s, || false).is_some(), "(struct S)1");
        assert!(cast(op(&int), &u, || true).is_none(), "GNU cast to union");
        assert!(
            cast(op(&int), &u, || false).is_some(),
            "(union U)x, no int member"
        );
        assert!(cast(op(&void), &int, || false).is_some(), "(int)v()");
        assert!(cast(op(&s), &void, || false).is_none(), "(void)s");
        assert!(binary(BinOp::Add, op(&s), op(&int)).is_some(), "s + 1");
        assert!(
            binary(BinOp::Add, op(&s), op(&s)).is_some(),
            "a + b on __m128i"
        );
        assert!(binary(BinOp::Eq, op(&s), op(&p)).is_some(), "s == p");
        assert!(binary(BinOp::Mul, op(&void), op(&int)).is_some(), "v() * 2");
        assert!(
            binary(BinOp::LogicalOr, op(&u), op(&int))
                .unwrap()
                .contains("union")
        );
        assert!(unary(UnaryOp::Neg, &s).is_some(), "-s");
        assert!(unary(UnaryOp::LogicalNot, &s).is_some(), "!s");
        assert!(unary(UnaryOp::BitNot, &void).is_some(), "~v()");
        assert!(increment(false, &s).is_some(), "s--");
        assert!(scalar_required(&s).is_some(), "if (s)");
        assert!(scalar_required(&void).is_some(), "if (v())");
        assert!(conditional_arms(&int, &s).is_some(), "x ? 0 : s");
        assert!(conditional_arms(&s, &s).is_none(), "x ? s : s");
        assert!(
            conditional_arms(&void, &int).is_none(),
            "GNU one-void-arm extension"
        );
        // A record against a vector is not judged (lccc's struct `__m128`).
        let v = CType::Vector(Box::new(CType::Float), 16);
        assert!(assignment(op(&s), &v).is_none() && assignment(op(&v), &s).is_none());
        assert!(binary(BinOp::Add, op(&s), op(&v)).is_none());
        assert!(cast(op(&s), &v, || false).is_none() && conditional_arms(&s, &v).is_none());
        // Ordinary operands are untouched, including GCC's leniencies.
        assert!(binary(BinOp::Add, op(&p), op(&int)).is_none(), "p + 1");
        assert!(
            binary(BinOp::Lt, op(&p), op(&int)).is_none(),
            "p < 1 (GCC warns)"
        );
        assert!(assignment(op(&int), &p).is_none(), "p = 1 (GCC permerror)");
    }
}
