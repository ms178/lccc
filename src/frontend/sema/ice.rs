//! Integer constant expressions (C11 6.6p6) where GNU C demands one: the
//! condition of `__builtin_choose_expr`.
//!
//! An ICE is a matter of *form*, not of foldability: `const int k = 1; k`
//! folds but is not one, nor is `2.0 > 1` or `(int)(2.0 > 1)`. Its operands
//! are integer, character and enumeration constants, `sizeof`/`_Alignof` of a
//! non-VLA, the GNU constant builtins, and floating constants only as the
//! *immediate* operand of a cast to an integer type. Every operand must have
//! that form, evaluated or not (`1 ? 2 : x` is not an ICE); what only
//! *evaluated* operands must also satisfy is a defined value: no division by
//! zero, no out-of-range shift count, no out-of-range floating conversion and
//! (from C99, GCC) no signed left shift that overflows or shifts a negative
//! value. Arithmetic overflow is accepted, as by both oracles.
//!
//! Every rule was measured against GCC 16.2 and Clang 23.1.3 (the table in
//! `tests/regression/check_sema_constraints.sh`). Where they split, lccc
//! follows GCC, which defines the builtin: signed shift overflow (Clang
//! accepts `1 << 31`), `(_Bool)1e300` (Clang rejects) and `INT_MIN / -1`
//! (Clang rejects).
//!
//! `__builtin_offsetof` is a macro expanding to `&((type *)0)->member` cast
//! to the reserved builtin typedef [`OFFSETOF_SIZE_TYPEDEF`]; only that cast
//! of an address is an ICE. The spelled-out idiom `(long)&((T *)0)->m` is
//! not, in either oracle -- and since the null pointer constant rule rests
//! on this test, accepting it changed types: GCC types `1 ? (void
//! *)((long)&((struct P *)0)->a) : (int *)0` as `void *`.

use crate::common::types::{CType, OFFSETOF_SIZE_TYPEDEF};
use crate::frontend::parser::ast::{
    BinOp, Expr, GenericAssociation, SizeofArg, TypeSpecifier, UnaryOp,
};
use crate::ir::reexports::IrConst;

/// What a named constant (C23 6.6p7) may contribute to an ICE: an integer
/// one is an operand, a floating one only the immediate operand of a cast to
/// an integer type (6.6p8).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum NamedConstant {
    Integer,
    Floating,
}

pub(super) struct IceChecker<'s> {
    /// The constant evaluator in GCC's `folding_initializer` mode:
    /// `__builtin_constant_p` answers now (0 unless proven), because a
    /// required constant expression is decided before any later pass could
    /// prove the operand constant.
    eval: &'s dyn Fn(&Expr) -> Option<IrConst>,
    ctype: &'s dyn Fn(&Expr) -> Option<CType>,
    /// The `_Generic` selection rule: the index of the selected association.
    select: &'s dyn Fn(&Expr, &[GenericAssociation]) -> Option<usize>,
    /// Whether an identifier or member access is a named constant: an
    /// enumeration constant in scope (not shadowed by an object), a C23
    /// `constexpr` object, or a member of one (`s.m`).
    named_constant: &'s dyn Fn(&Expr) -> Option<NamedConstant>,
    /// Whether `name` is a builtin function.
    is_builtin: &'s dyn Fn(&str) -> bool,
    /// C90/C94: a signed left shift that overflows is still a constant (GCC
    /// diagnoses it as not constant only from C99, `flag_isoc99`).
    pre_c99: bool,
}

impl<'s> IceChecker<'s> {
    pub(super) fn new(
        eval: &'s dyn Fn(&Expr) -> Option<IrConst>,
        ctype: &'s dyn Fn(&Expr) -> Option<CType>,
        select: &'s dyn Fn(&Expr, &[GenericAssociation]) -> Option<usize>,
        named_constant: &'s dyn Fn(&Expr) -> Option<NamedConstant>,
        is_builtin: &'s dyn Fn(&str) -> bool,
        pre_c99: bool,
    ) -> Self {
        Self {
            eval,
            ctype,
            select,
            named_constant,
            is_builtin,
            pre_c99,
        }
    }

    /// Whether `expr` is an integer constant expression with a defined value.
    pub(super) fn is_ice(&self, expr: &Expr) -> bool {
        self.form(expr, true) && (self.eval)(expr).is_some_and(|c| c.to_i128().is_some())
    }

    fn int_value(&self, expr: &Expr) -> Option<i128> {
        (self.eval)(expr)?.to_i128()
    }

    /// Whether `expr` has the form of an ICE; `evaluated` says whether its
    /// value is computed (an unselected `?:` arm, the right operand of a
    /// decided `&&`/`||` and an unselected `__builtin_choose_expr` arm are
    /// not), so value-only rules apply to it.
    fn form(&self, expr: &Expr, evaluated: bool) -> bool {
        match expr {
            Expr::IntLiteral(..)
            | Expr::UIntLiteral(..)
            | Expr::LongLiteral(..)
            | Expr::ULongLiteral(..)
            | Expr::LongLongLiteral(..)
            | Expr::ULongLongLiteral(..)
            | Expr::CharLiteral(..) => true,
            Expr::Identifier(..) | Expr::MemberAccess(..) => {
                (self.named_constant)(expr) == Some(NamedConstant::Integer)
            }
            Expr::UnaryOp(
                UnaryOp::Plus
                | UnaryOp::Neg
                | UnaryOp::BitNot
                | UnaryOp::LogicalNot
                | UnaryOp::RealPart
                | UnaryOp::ImagPart,
                inner,
                _,
            ) => self.form(inner, evaluated),
            Expr::BinaryOp(op @ (BinOp::LogicalAnd | BinOp::LogicalOr), lhs, rhs, _) => {
                // A decided left operand leaves the right one unevaluated.
                let rhs_evaluated = evaluated
                    && self
                        .int_value(lhs)
                        .is_none_or(|v| (v != 0) == matches!(op, BinOp::LogicalAnd));
                self.form(lhs, evaluated) && self.form(rhs, rhs_evaluated)
            }
            Expr::BinaryOp(op, lhs, rhs, _) => {
                self.form(lhs, evaluated)
                    && self.form(rhs, evaluated)
                    && (!evaluated || self.defined(*op, lhs, rhs))
            }
            Expr::Conditional(cond, then_e, else_e, _) => {
                let taken = self.int_value(cond).map(|v| v != 0);
                self.form(cond, evaluated)
                    && self.form(then_e, evaluated && taken != Some(false))
                    && self.form(else_e, evaluated && taken != Some(true))
            }
            Expr::GnuConditional(cond, else_e, _) => {
                let taken = self.int_value(cond).map(|v| v != 0);
                self.form(cond, evaluated) && self.form(else_e, evaluated && taken != Some(true))
            }
            Expr::Cast(_, inner, _, _) => self.cast_form(expr, inner, evaluated),
            Expr::Sizeof(arg, _) => match arg.as_ref() {
                SizeofArg::Type(ts) => self.non_vla_type_name(ts),
                // A VLA operand has no constant size; the evaluator refuses
                // it, which `is_ice` requires.
                SizeofArg::Expr(_) => true,
            },
            Expr::Countof(arg, _) => match arg.as_ref() {
                SizeofArg::Type(TypeSpecifier::Array(_, Some(bound))) => self.is_ice(bound),
                SizeofArg::Type(ts) => self.non_vla_type_name(ts),
                SizeofArg::Expr(_) => true,
            },
            Expr::Alignof(..)
            | Expr::AlignofExpr(..)
            | Expr::GnuAlignof(..)
            | Expr::GnuAlignofExpr(..)
            | Expr::BuiltinTypesCompatibleP(..) => true,
            Expr::GenericSelection(controlling, associations, _) => {
                (self.select)(controlling, associations)
                    .and_then(|i| associations.get(i))
                    .is_some_and(|a| self.form(&a.expr, evaluated))
            }
            Expr::FunctionCall(callee, args, _) => {
                let Expr::Identifier(name, _) = callee.as_ref() else {
                    return false;
                };
                self.builtin_call_form(expr, name, args, evaluated)
            }
            _ => false,
        }
    }

    fn builtin_call_form(&self, call: &Expr, name: &str, args: &[Expr], evaluated: bool) -> bool {
        match name {
            "__builtin_choose_expr" if args.len() == 3 => {
                let Some(c) = self.int_value(&args[0]) else {
                    return false;
                };
                // The unselected arm is not part of the expression at all:
                // any form (GCC and Clang alike).
                let chosen = if c != 0 { &args[1] } else { &args[2] };
                self.form(&args[0], evaluated) && self.form(chosen, evaluated)
            }
            // Any operand: the answer itself is the constant.
            "__builtin_constant_p" | "__builtin_classify_type" => true,
            "__builtin_expect" | "__builtin_expect_with_probability" => {
                args.len() >= 2 && self.form(&args[0], evaluated) && self.form(&args[1], evaluated)
            }
            // A folding builtin with an integer result (`__builtin_popcount(3)`,
            // `__builtin_strlen("ab")`, `__builtin_isnan(1.0)`): its value
            // decides, so the operands need not be ICEs themselves; one that
            // does not fold (`__builtin_expect(x, 1)`) is refused by `is_ice`.
            _ if (self.is_builtin)(name) => {
                (self.eval)(call).is_some_and(|c| c.to_i128().is_some())
            }
            _ => false,
        }
    }

    fn cast_form(&self, cast: &Expr, inner: &Expr, evaluated: bool) -> bool {
        let Some(target) = (self.ctype)(cast) else {
            return false;
        };
        if !target.is_integer() {
            return false;
        }
        if let Some(value) = floating_literal(inner) {
            return !evaluated || float_converts(value, &target);
        }
        if (self.named_constant)(inner) == Some(NamedConstant::Floating) {
            let value = (self.eval)(inner).and_then(|c| c.to_f64());
            return !evaluated || value.is_some_and(|v| float_converts(v, &target));
        }
        if matches!(inner, Expr::AddressOf(..)) {
            // `__builtin_offsetof` (see the module comment).
            let offsetof_expansion = matches!(cast, Expr::Cast(TypeSpecifier::TypedefName(name), ..)
                if name == OFFSETOF_SIZE_TYPEDEF);
            return offsetof_expansion && (self.eval)(cast).is_some();
        }
        self.form(inner, evaluated)
    }

    /// Value rules for an evaluated binary operation on ICE operands.
    fn defined(&self, op: BinOp, lhs: &Expr, rhs: &Expr) -> bool {
        match op {
            BinOp::Div | BinOp::Mod => self.int_value(rhs) != Some(0),
            BinOp::Shl | BinOp::Shr => {
                let Some(lhs_ty) = (self.ctype)(lhs) else {
                    return true;
                };
                let lhs_ty = lhs_ty.integer_promoted();
                let width = (lhs_ty.size() * 8) as i128;
                let Some(count) = self.int_value(rhs) else {
                    return true;
                };
                if count < 0 || count >= width {
                    return false;
                }
                if op == BinOp::Shr || !lhs_ty.is_signed() || self.pre_c99 {
                    return true;
                }
                let Some(value) = self.int_value(lhs) else {
                    return true;
                };
                let max = if width >= 128 {
                    i128::MAX
                } else {
                    (1i128 << (width - 1)) - 1
                };
                value >= 0 && value <= (max >> count)
            }
            _ => true,
        }
    }

    /// A `sizeof` type name: every array bound must itself be an ICE, or
    /// the type is a VLA (`sizeof(char[2.5 > 1 ? 2 : 3])` is not constant).
    fn non_vla_type_name(&self, ts: &TypeSpecifier) -> bool {
        match ts {
            TypeSpecifier::Array(elem, bound) => {
                bound.as_deref().is_none_or(|b| self.is_ice(b)) && self.non_vla_type_name(elem)
            }
            _ => true,
        }
    }
}

/// The value of a floating constant, when `expr` is one. A negated literal
/// is an expression, not a constant (`(int)-2.5` is no ICE); a decimal
/// floating constant converts only through its own evaluator, which this
/// rule does not model, so it is refused.
fn floating_literal(expr: &Expr) -> Option<f64> {
    match expr {
        Expr::FloatLiteral(v, _)
        | Expr::FloatLiteralF32(v, _)
        | Expr::FloatLiteralLongDouble(v, _, _)
        | Expr::FloatLiteralF128(v, _, _) => Some(*v),
        _ => None,
    }
}

/// Whether converting floating `value` to integer `target` is defined (C11
/// 6.3.1.4p1: the truncated value must be representable); every value,
/// NaN included, converts to `_Bool` (C11 6.3.1.2: it compares unequal to 0;
/// GCC 16.2 accepts `(_Bool)1e300`).
fn float_converts(value: f64, target: &CType) -> bool {
    if matches!(target, CType::Bool) {
        return true;
    }
    if !value.is_finite() {
        return false;
    }
    let t = value.trunc();
    let bits = (target.size() * 8) as i32;
    let (lo, hi) = if target.is_signed() {
        (-(2f64.powi(bits - 1)), 2f64.powi(bits - 1))
    } else {
        (0.0, 2f64.powi(bits))
    };
    t >= lo && t < hi
}
