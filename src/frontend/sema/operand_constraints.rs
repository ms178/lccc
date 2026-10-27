//! Operand-type constraints of C23 6.5, 6.5.2.3 and 6.5.3.2 and of the
//! "as if by assignment" conversions of 6.5.16.1: the operand kinds that have
//! no valid use as arithmetic or truth values (structures and unions, `void`,
//! C23 `nullptr_t`), the operand class of `*`, `.` and `->`, and the two
//! `void` value verdicts.
//!
//! lccc used to accept these operands and hand them to lowering, which has no
//! meaning for them: `a + b` on two (struct-based) `__m128i` values compiled
//! into a binary that segfaulted, and `*x` on an `int`, `x->a` on a `double`
//! or `p.a` on a pointer were compiled as whatever the lowering guessed, where
//! GCC 16.2 and Clang 23.1.4 reject the program. Each rule below is a
//! constraint the oracles enforce, with the messages GCC uses.
//!
//! The rules are pure functions of the operand types -- some of them the
//! *undecayed* type (`.` does not apply array-to-pointer conversion; see
//! `SemanticAnalyzer::exact_operand_ctype`) -- and of whether an operand is a
//! null pointer constant, so they are unit-tested here without building
//! expressions. The analyzer decides which inferred types are exact enough to
//! check, owns the column policy, and owns every message that quotes the
//! operator's operand; every other combination, such as GCC's pointer/integer
//! leniency, is outside this module and unchanged.

use crate::common::types::{AddressSpace, CType};
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

pub(super) const VOID_VALUE: &str = "void value not ignored as it ought to be";

/// The two record verdicts of [`scalar_required`]: a record operand where a
/// truth value is required.
pub(super) const STRUCT_VALUE: &str = "used struct type value where scalar is required";
pub(super) const UNION_VALUE: &str = "used union type value where scalar is required";

/// GCC's other `void` diagnostic: an operator that would have to *produce* a
/// value from the operand (`!`, `~`, `-`, `+`, `++`, `--`, `op=`, a cast) says
/// this instead of the "not ignored as it ought to be" form that the value
/// contexts (initialization, `return`, argument passing, binary operators,
/// conditions) use. Both were measured on gcc-16.2.
const VOID_USE: &str = "invalid use of void expression";

/// GCC 16.2's spelling of a type in a diagnostic: `type_to_string`'s form,
/// which differs from C's declaration syntax in ways that matter for
/// verbatim message parity -- `long unsigned int` (not `unsigned long`),
/// `long long int`, `__int128 unsigned`, `complex double` (not
/// `_Complex double`), and `__vector(4) int` for a GNU vector. A typedef
/// name cannot be reproduced (the inferred type no longer knows it), so
/// GCC's `'T' {aka 'int'}` appears here as plain `'int'`.
/// The parameter list of a function type in gcc's spelling: `void` for an
/// empty prototype, `int, char *` otherwise, `...` appended when variadic.
fn function_params(f: &crate::common::types::FunctionType) -> String {
    let mut params: Vec<String> = f.params.iter().map(|(ty, _)| type_name(ty)).collect();
    if params.is_empty() && f.has_prototype {
        params.push("void".into());
    }
    if f.variadic {
        params.push("...".into());
    }
    params.join(", ")
}

pub(super) fn type_name(ty: &CType) -> String {
    match ty {
        CType::Long => "long int".into(),
        CType::ULong => "long unsigned int".into(),
        CType::LongLong => "long long int".into(),
        CType::ULongLong => "long long unsigned int".into(),
        CType::Int128 => "__int128".into(),
        CType::UInt128 => "__int128 unsigned".into(),
        CType::ComplexFloat => "complex float".into(),
        CType::ComplexDouble => "complex double".into(),
        CType::ComplexLongDouble => "complex long double".into(),
        // `type_to_string` writes a function type without the space its
        // declaration syntax has (`int(void)`, not `int (void)`), a pointer
        // to an array as `int (*)[3]`, and a pointer to a function as
        // `int (*)(void)`.
        CType::Function(f) => {
            let mut spelling = type_name(&f.return_type);
            spelling.push('(');
            spelling.push_str(&function_params(f));
            spelling.push(')');
            spelling
        }
        CType::Pointer(inner, AddressSpace::Default) => match &**inner {
            CType::Array(elem, len) => format!(
                "{} (*)[{}]",
                type_name(elem),
                len.map(|n| n.to_string()).unwrap_or_default()
            ),
            CType::Function(f) => {
                format!("{} (*)({})", type_name(&f.return_type), function_params(f))
            }
            _ => format!("{} *", type_name(inner)),
        },
        CType::Vector(elem, total_size) => {
            let lanes = match elem.size() {
                0 => 1,
                size => total_size / size,
            };
            format!("__vector({lanes}) {}", type_name(elem))
        }
        _ => ty.to_string(),
    }
}

/// GCC's "used struct type value where scalar is required", for a controlling
/// expression, `!`, `&&`, `||` or the first operand of `?:`. `nullptr_t` is a
/// scalar type and allowed (it converts to `false`).
pub(super) fn scalar_required(ty: &CType) -> Option<String> {
    match ty {
        CType::Struct(_) => Some(STRUCT_VALUE.into()),
        CType::Union(_) => Some(UNION_VALUE.into()),
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
        Kind::Void => Some(VOID_USE.into()),
        Kind::Record => Some(format!("wrong type argument to {what}")),
        // `!nullptr` is valid (a scalar operand); every arithmetic unary
        // operator requires an arithmetic operand (C23 6.5.3.3p1).
        Kind::NullPtr if op != UnaryOp::LogicalNot => {
            Some(format!("wrong type argument to {what}"))
        }
        _ => None,
    }
}

/// Unary `*` (C23 6.5.3.2p2): the operand must have pointer type. An array or
/// a function designator is accepted -- both convert to a pointer, and `*f`
/// is the designator `f` itself. A `void` operand has no value to speak of,
/// so its diagnostic is GCC's void-value error rather than a type-argument
/// message; everything else (arithmetic, record, vector, `nullptr_t`) is
/// "invalid type argument of unary '*'".
pub(super) fn indirection(ty: &CType) -> Option<String> {
    match kind(ty) {
        Kind::Pointer => None,
        Kind::Void => Some(VOID_VALUE.into()),
        // The message quotes the *promoted* operand type: `*c` on a
        // `char`/`_Bool` says `'int'`, and an enumeration its compatible
        // type (`'unsigned int'` for `enum E { A }`), measured on gcc-16.2.
        _ => Some(format!(
            "invalid type argument of unary '*' (have '{}')",
            type_name(&ty.integer_promoted())
        )),
    }
}

/// Whether `ty` is a pointer to `void`. Dereferencing one is a GNU extension
/// GCC accepts with the default-on warning `dereferencing 'void *' pointer`;
/// the resulting lvalue has type `void`, which is what the value rules judge.
pub(super) fn is_void_pointer(ty: &CType) -> bool {
    matches!(ty, CType::Pointer(inner, _) if matches!(**inner, CType::Void))
}

/// What a member selection can do with its operand, before the member name is
/// looked up (C23 6.5.2.3p2-p3). The analyzer owns the messages that name the
/// member or quote the operand, so this only classifies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum MemberOperand {
    /// A struct or union, reached directly (`.`) or through a pointer (`->`).
    Record,
    /// `.` on a pointer to a record: GCC asks whether `->` was meant.
    Pointer,
    /// `->` on a pointer to a pointer to a record: GCC asks whether it should
    /// be dereferenced first.
    PointerToPointer,
    /// `->` on a value that is not a pointer at all.
    NotPointer,
    /// `->` through `void *`: GCC warns and then rejects the member request.
    VoidPointer,
    /// A value no member can be selected from.
    NotRecord,
}

/// Classify the operand type of `.` (`arrow == false`) or `->`
/// (`arrow == true`). An array or function operand of `->` has already
/// converted to a pointer (C23 6.3.2.1p3), so it is judged by what it points
/// to; the same conversion makes `int a[3]; a->x` a member request on `int`,
/// which GCC rejects.
pub(super) fn member_operand(ty: &CType, arrow: bool) -> MemberOperand {
    if arrow {
        return match ty {
            CType::Pointer(inner, _) | CType::Array(inner, _) => match &**inner {
                // An ext vector has named lanes, so `p->x` on an `I4 *` is a
                // lane selection, not a member error (the analyzer validates
                // the selector).
                CType::Struct(_) | CType::Union(_) | CType::ExtVector(..) => MemberOperand::Record,
                CType::Void => MemberOperand::VoidPointer,
                // `struct S **pp; pp->a`: the pointee is itself a pointer to
                // a record, which GCC suggests dereferencing first. A pointer
                // to a non-record (`int **`) gets the plain member error.
                CType::Pointer(pointee, _)
                    if matches!(&**pointee, CType::Struct(_) | CType::Union(_)) =>
                {
                    MemberOperand::PointerToPointer
                }
                _ => MemberOperand::NotRecord,
            },
            // A function designator decays to a pointer to the function itself.
            CType::Function(_) => MemberOperand::NotRecord,
            _ => MemberOperand::NotPointer,
        };
    }
    match ty {
        // An ext vector has named lanes; the analyzer validates the selector.
        CType::Struct(_) | CType::Union(_) | CType::ExtVector(..) => MemberOperand::Record,
        // gcc-16.2 suggests `->` only when the pointer's pointee is a record;
        // `int *`, `void *` and `struct S **` get the plain member error.
        CType::Pointer(pointee, _)
            if matches!(
                &**pointee,
                CType::Struct(_) | CType::Union(_) | CType::ExtVector(..)
            ) =>
        {
            MemberOperand::Pointer
        }
        _ => MemberOperand::NotRecord,
    }
}

/// Postfix and prefix `++`/`--` (C23 6.5.2.4p1, 6.5.3.1p1): a real or pointer
/// operand.
pub(super) fn increment(increment: bool, ty: &CType) -> Option<String> {
    let what = if increment { "increment" } else { "decrement" };
    match kind(ty) {
        Kind::Void => Some(VOID_USE.into()),
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
        // Two records convert only when they are the same record type
        // (gcc-16.2: `1 ? s : t` for `struct S`/`struct T` and for a
        // `struct` against a `union` is "type mismatch in conditional
        // expression"; an unnamed record is a different type from every
        // other declaration too).
        (Kind::Record, Kind::Record) => then_ty == else_ty,
        (Kind::Record, _) | (_, Kind::Record) => false,
        (Kind::NullPtr, other) | (other, Kind::NullPtr) => {
            matches!(other, Kind::NullPtr | Kind::Pointer)
        }
        _ => true,
    };
    (!valid).then(|| "type mismatch in conditional expression".to_string())
}

/// Where an "as if by assignment" conversion happens (C23 6.5.16.1p1). GCC
/// words the record mismatch differently in each context, and it uses both of
/// its `void` diagnostics here (measured on gcc-16.2): initialization and
/// `return` say "void value not ignored as it ought to be" at the operand,
/// while assignment and argument passing say "invalid use of void expression".
#[derive(Clone, Copy, Debug)]
pub(super) enum AssignmentCtx<'a> {
    Initialization,
    /// `=` itself: the diagnostic is anchored at the assignment operator.
    Assignment,
    Returning,
    Argument {
        index: usize,
        callee: &'a str,
    },
}

/// Simple assignment, and the conversions "as if by assignment" of
/// initialization, argument passing and `return` (C23 6.5.16.1p1), for a
/// `nullptr_t`, `void` or record operand on either side. Record-to-record
/// compatibility stays with the callers, which know the anonymous-record key
/// caveats.
pub(super) fn assignment(from: Operand<'_>, to: &CType, ctx: AssignmentCtx<'_>) -> Option<String> {
    let (fk, tk) = (kind(from.ty), kind(to));
    let valid = match (fk, tk) {
        (Kind::Void, _) => {
            return Some(match ctx {
                AssignmentCtx::Initialization | AssignmentCtx::Returning => VOID_VALUE.into(),
                AssignmentCtx::Assignment | AssignmentCtx::Argument { .. } => VOID_USE.into(),
            });
        }
        // An object of type `void` cannot exist; nothing to diagnose here.
        (_, Kind::Void) => true,
        _ if record_vector_pair(fk, tk) => true,
        (Kind::Record, Kind::Record) => true,
        (Kind::Record, _) | (_, Kind::Record) => false,
        (_, Kind::NullPtr) => fk == Kind::NullPtr || from.null_constant,
        (Kind::NullPtr, _) => tk == Kind::Pointer || matches!(to, CType::Bool),
        _ => true,
    };
    (!valid).then(|| match ctx {
        AssignmentCtx::Initialization => format!(
            "incompatible types when initializing type '{to}' using type '{}'",
            from.ty
        ),
        AssignmentCtx::Assignment => format!(
            "incompatible types when assigning to type '{to}' from type '{}'",
            from.ty
        ),
        AssignmentCtx::Returning => format!(
            "incompatible types when returning type '{}' but '{to}' was expected",
            from.ty
        ),
        AssignmentCtx::Argument { index, callee } => {
            format!("incompatible type for argument {} of '{callee}'", index + 1)
        }
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
        Kind::Void => return Some(VOID_USE.into()),
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
    // C11 6.5.4p2: a pointer converts only to an integer type. GCC 16.2
    // rejects pointer-to-floating/complex casts with this exact wording,
    // and `-w` does not silence a constraint (measured).
    if fk == Kind::Pointer {
        if to.is_floating() {
            return Some("pointer value used where a floating-point was expected".into());
        }
        if to.is_complex() {
            return Some("pointer value used where a complex was expected".into());
        }
    }
    if kind(to) == Kind::Pointer && (from.ty.is_floating() || from.ty.is_complex()) {
        return Some("cannot convert to a pointer type".into());
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
    use crate::common::types::{AddressSpace, EnumType};

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
        assert!(
            assignment(op(&n), &int, AssignmentCtx::Assignment).is_some(),
            "int a = nullptr"
        );
        assert!(
            assignment(op(&n), &dbl, AssignmentCtx::Assignment).is_some(),
            "double a = nullptr"
        );
        assert!(
            assignment(op(&p), &n, AssignmentCtx::Assignment).is_some(),
            "nullptr_t m = p"
        );
        assert!(
            assignment(op(&n), &p, AssignmentCtx::Assignment).is_none(),
            "int *q = n"
        );
        assert!(
            assignment(op(&n), &bool_, AssignmentCtx::Assignment).is_none(),
            "bool b = n"
        );
        assert!(
            assignment(npc(&int), &n, AssignmentCtx::Assignment).is_none(),
            "nullptr_t m = 0"
        );
        assert!(
            assignment(op(&int), &n, AssignmentCtx::Assignment).is_some(),
            "nullptr_t m = x"
        );
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
        assert!(
            assignment(op(&s), &int, AssignmentCtx::Assignment).is_some(),
            "int a = s"
        );
        assert!(
            assignment(op(&int), &s, AssignmentCtx::Assignment).is_some(),
            "s = 1"
        );
        assert!(
            assignment(op(&s), &s, AssignmentCtx::Assignment).is_none(),
            "record pairs are the caller's"
        );
        assert!(
            assignment(op(&void), &int, AssignmentCtx::Assignment).is_some(),
            "int a = v()"
        );
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
        // C11 6.5.4p2, both directions (measured on GCC 16.2.0).
        assert_eq!(
            cast(op(&p), &CType::Double, || false).as_deref(),
            Some("pointer value used where a floating-point was expected"),
            "(double)p"
        );
        assert_eq!(
            cast(op(&CType::Double), &p, || false).as_deref(),
            Some("cannot convert to a pointer type"),
            "(char *)b"
        );
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
        assert!(
            assignment(op(&s), &v, AssignmentCtx::Assignment).is_none()
                && assignment(op(&v), &s, AssignmentCtx::Assignment).is_none()
        );
        assert!(binary(BinOp::Add, op(&s), op(&v)).is_none());
        assert!(cast(op(&s), &v, || false).is_none() && conditional_arms(&s, &v).is_none());
        // Ordinary operands are untouched, including GCC's leniencies.
        assert!(binary(BinOp::Add, op(&p), op(&int)).is_none(), "p + 1");
        assert!(
            binary(BinOp::Lt, op(&p), op(&int)).is_none(),
            "p < 1 (GCC warns)"
        );
        assert!(
            assignment(op(&int), &p, AssignmentCtx::Assignment).is_none(),
            "p = 1 (GCC permerror)"
        );
    }

    /// `*` (C23 6.5.3.2p2) and the operand classes of `.`/`->` (6.5.2.3p2-p3),
    /// measured one statement at a time on gcc-16.2; Clang 23.1.4 agrees on
    /// every verdict (it words the messages differently, so the text here is
    /// GCC's). Enumerations are covered end to end by
    /// `tests/regression/check_operand_diagnostics.sh`.
    #[test]
    fn indirection_and_member_operands_match_gcc() {
        let (int, dbl, void) = (CType::Int, CType::Double, CType::Void);
        let s = CType::Struct("S".into());
        let u = CType::Union("U".into());
        let vec = CType::Vector(Box::new(CType::Int), 16);
        let p = ptr(CType::Int);
        let arr = CType::Array(Box::new(CType::Int), Some(3));
        let func = CType::Function(Box::new(crate::common::types::FunctionType {
            return_type: CType::Int,
            params: Vec::new(),
            variadic: false,
            has_prototype: true,
        }));
        // Anything with pointer type is accepted; an array and a function
        // designator convert to one, so `*a` and `*g` are legal.
        assert!(indirection(&p).is_none(), "*p");
        assert!(indirection(&arr).is_none(), "*a");
        assert!(indirection(&func).is_none(), "*g");
        // `void` operands get the value error; the rest GCC's type argument
        // message, quoting the *integer-promoted* operand type.
        assert_eq!(indirection(&void).as_deref(), Some(VOID_VALUE));
        let msg = |ty: &CType| indirection(ty).unwrap();
        assert_eq!(msg(&int), "invalid type argument of unary '*' (have 'int')");
        assert_eq!(
            msg(&dbl),
            "invalid type argument of unary '*' (have 'double')"
        );
        assert_eq!(
            msg(&CType::Bool),
            "invalid type argument of unary '*' (have 'int')"
        );
        assert_eq!(
            msg(&CType::Short),
            "invalid type argument of unary '*' (have 'int')"
        );
        assert_eq!(
            msg(&CType::Float),
            "invalid type argument of unary '*' (have 'float')"
        );
        assert_eq!(
            msg(&CType::ComplexDouble),
            "invalid type argument of unary '*' (have 'complex double')"
        );
        assert_eq!(
            msg(&vec),
            "invalid type argument of unary '*' (have '__vector(4) int')"
        );
        assert_eq!(
            msg(&s),
            "invalid type argument of unary '*' (have 'struct S')"
        );
        assert_eq!(
            msg(&u),
            "invalid type argument of unary '*' (have 'union U')"
        );
        // `->` requires a pointer to a record; `.` requires a record.
        assert_eq!(member_operand(&s, false), MemberOperand::Record);
        assert_eq!(member_operand(&u, false), MemberOperand::Record);
        assert_eq!(member_operand(&ptr(s.clone()), true), MemberOperand::Record);
        assert_eq!(member_operand(&ptr(u.clone()), true), MemberOperand::Record);
        // A `.` on `int *` is the plain member error; the suggestion appears
        // only when the pointer's pointee is a record.
        assert_eq!(member_operand(&p, false), MemberOperand::NotRecord);
        assert_eq!(
            member_operand(&ptr(s.clone()), false),
            MemberOperand::Pointer
        );
        assert_eq!(
            member_operand(&ptr(ptr(s.clone())), false),
            MemberOperand::NotRecord
        );
        assert_eq!(member_operand(&p, true), MemberOperand::NotRecord);
        assert_eq!(
            member_operand(&ptr(ptr(s.clone())), true),
            MemberOperand::PointerToPointer
        );
        assert_eq!(
            member_operand(&ptr(ptr(CType::Int)), true),
            MemberOperand::NotRecord
        );
        // An ext vector's lanes are selected, not looked up as members, so a
        // pointer to one is accepted here and validated by the selector check.
        let ext = CType::ExtVector(
            Box::new(CType::Int),
            crate::common::types::ExtVectorLayout::new(&CType::Int, 4).unwrap(),
        );
        assert_eq!(member_operand(&ext, false), MemberOperand::Record);
        assert_eq!(
            member_operand(&ptr(ext.clone()), true),
            MemberOperand::Record
        );
        assert_eq!(member_operand(&ptr(ext), false), MemberOperand::Pointer);
        assert_eq!(
            member_operand(&ptr(ptr(void.clone())), true),
            MemberOperand::NotRecord
        );
        assert_eq!(
            member_operand(&ptr(void.clone()), true),
            MemberOperand::VoidPointer
        );
        assert_eq!(member_operand(&arr, false), MemberOperand::NotRecord);
        assert_eq!(member_operand(&arr, true), MemberOperand::NotRecord);
        assert_eq!(member_operand(&func, true), MemberOperand::NotRecord);
        assert_eq!(member_operand(&int, false), MemberOperand::NotRecord);
        assert_eq!(member_operand(&int, true), MemberOperand::NotPointer);
        assert_eq!(member_operand(&dbl, true), MemberOperand::NotPointer);
        assert_eq!(member_operand(&s, true), MemberOperand::NotPointer);
        // `void *` is a pointer, so `*p` warns (GNU extension) and yields the
        // `void` the value rules then judge.
        assert!(is_void_pointer(&ptr(void.clone())));
        assert!(!is_void_pointer(&p));
        assert!(!is_void_pointer(&ptr(ptr(void.clone()))));
        assert!(!is_void_pointer(&void));
    }

    /// The quoted type spelling of the operand messages is gcc-16.2's
    /// `type_to_string`, not C declaration syntax.
    #[test]
    fn quoted_types_use_gcc_spelling() {
        assert_eq!(type_name(&CType::Int), "int");
        assert_eq!(type_name(&CType::UInt), "unsigned int");
        assert_eq!(type_name(&CType::Long), "long int");
        assert_eq!(type_name(&CType::ULong), "long unsigned int");
        assert_eq!(type_name(&CType::LongLong), "long long int");
        assert_eq!(type_name(&CType::ULongLong), "long long unsigned int");
        assert_eq!(type_name(&CType::Int128), "__int128");
        assert_eq!(type_name(&CType::UInt128), "__int128 unsigned");
        assert_eq!(type_name(&CType::Float), "float");
        assert_eq!(type_name(&CType::Float16), "_Float16");
        assert_eq!(type_name(&CType::ComplexFloat), "complex float");
        assert_eq!(type_name(&CType::ComplexLongDouble), "complex long double");
        assert_eq!(
            type_name(&CType::Vector(Box::new(CType::Int), 16)),
            "__vector(4) int"
        );
        assert_eq!(
            type_name(&CType::Vector(Box::new(CType::Double), 32)),
            "__vector(4) double"
        );
        assert_eq!(type_name(&ptr(CType::Int)), "int *");
        assert_eq!(
            type_name(&ptr(CType::Array(Box::new(CType::Int), Some(3)))),
            "int (*)[3]"
        );
        let f = CType::Function(Box::new(crate::common::types::FunctionType {
            return_type: CType::Int,
            params: vec![(CType::Int, None)],
            variadic: false,
            has_prototype: true,
        }));
        assert_eq!(type_name(&f), "int(int)");
        assert_eq!(type_name(&ptr(f)), "int (*)(int)");
        assert_eq!(
            type_name(&CType::Function(Box::new(
                crate::common::types::FunctionType {
                    return_type: CType::Void,
                    params: Vec::new(),
                    variadic: true,
                    has_prototype: false,
                }
            ))),
            "void(...)"
        );
        assert_eq!(type_name(&CType::Struct("S".into())), "struct S");
        assert_eq!(
            type_name(&CType::Struct("__anon_struct_7".into())),
            "struct <anonymous>"
        );
        // An enumeration quotes its compatible type once promoted, so the
        // enum spelling itself only matters for a bare `type_name` call.
        let e = CType::Enum(EnumType {
            name: Some("E".into()),
            variants: vec![("A".into(), 0)],
            is_packed: false,
            fixed: None,
        });
        assert_eq!(type_name(&e), "enum E");
        assert_eq!(type_name(&e.integer_promoted()), "unsigned int");
    }

    /// GCC words an incompatible record conversion differently in each
    /// context, and uses both of its `void` diagnostics (measured).
    #[test]
    fn conversions_are_worded_per_context() {
        let (int, void) = (CType::Int, CType::Void);
        let s = CType::Struct("S".into());
        let arg = || AssignmentCtx::Argument {
            index: 0,
            callee: "h",
        };
        assert_eq!(
            assignment(op(&s), &int, AssignmentCtx::Initialization).unwrap(),
            "incompatible types when initializing type 'int' using type 'struct S'"
        );
        assert_eq!(
            assignment(op(&s), &int, AssignmentCtx::Assignment).unwrap(),
            "incompatible types when assigning to type 'int' from type 'struct S'"
        );
        assert_eq!(
            assignment(op(&s), &int, AssignmentCtx::Returning).unwrap(),
            "incompatible types when returning type 'struct S' but 'int' was expected"
        );
        assert_eq!(
            assignment(op(&s), &int, arg()).unwrap(),
            "incompatible type for argument 1 of 'h'"
        );
        assert_eq!(
            assignment(op(&void), &int, AssignmentCtx::Initialization).unwrap(),
            VOID_VALUE
        );
        assert_eq!(
            assignment(op(&void), &int, AssignmentCtx::Returning).unwrap(),
            VOID_VALUE
        );
        assert_eq!(
            assignment(op(&void), &int, AssignmentCtx::Assignment).unwrap(),
            VOID_USE
        );
        assert_eq!(assignment(op(&void), &int, arg()).unwrap(), VOID_USE);
        // A matching record pair converts (C23 6.5.16.1p1) and so does a
        // record-to-record copy of the same type.
        assert!(assignment(op(&s), &s, AssignmentCtx::Assignment).is_none());
    }

    /// Two records in `?:` convert only when they are the same type.
    #[test]
    fn conditional_rejects_distinct_records() {
        let (s, t, u) = (
            CType::Struct("S".into()),
            CType::Struct("T".into()),
            CType::Union("U".into()),
        );
        assert!(conditional_arms(&s, &s).is_none());
        assert!(conditional_arms(&s, &t).is_some());
        assert!(conditional_arms(&s, &u).is_some());
        assert!(conditional_arms(&s, &CType::Int).is_some());
    }
}
