//! `-Wreturn-local-addr`: a `return` whose value is the address of an object
//! that dies with the function (an automatic variable or a parameter), or of
//! a label.
//!
//! The rule is GCC 16.2's front-end walk (`c_finish_return`), measured case
//! by case against `gcc-16 -fsyntax-only` (the walk is identical at every
//! optimization level; see `tests/regression/fixtures/return_local_addr/`):
//!
//! * the function must return a pointer (`long f(void) { int x; return
//!   (long)&x; }` is silent);
//! * the walk strips conversions (explicit casts included, even through an
//!   integer: `(int *)(long)&x` warns; a conversion to `_Bool` is a
//!   comparison, not a conversion, and stops it), pointer `+`/`-` (to the
//!   pointer operand), and integer `+`/`-`. Integer `+` follows GCC's operand
//!   canonicalization: a literal constant goes second, then an
//!   address-invariant operand goes second, so `(int *)(4 + (long)&x)` warns
//!   while `(int *)((long)&x + n)` does not. An integer `-` whose right
//!   operand is (a conversion of) a pointer stops the walk, as GCC's does;
//!   a pointer difference is not followed either, except `p - (T *)0`, which
//!   folds to a conversion of `p`;
//! * the folding GCC applies before the walk: a comma whose left operand has
//!   no side effects is its right operand; a conditional (or GNU `?:`) with a
//!   condition of known truth is the selected arm, and one with identical
//!   arms and a side-effect-free condition is that arm. A condition's truth
//!   is known for integer constant expressions and for addresses that cannot
//!   be null -- string literals, automatic and parameter objects and
//!   non-weak named objects (`&g ? &x : 0` warns, a weak `&w ? &x : 0` does
//!   not). `_Generic` and `__builtin_choose_expr` are their selected
//!   operand;
//! * at an address -- `&` or an array that decays -- the walk descends
//!   through members, array subscripts of array objects, `__real__`/
//!   `__imag__` and the folded `*&e` and `(&e)->m`, but never through a
//!   pointer dereference (`&ps->m` and `&p[0]` are silent), and warns when it
//!   reaches an automatic object of the current function. A nested
//!   function returning its parent's local is silent (the object outlives the
//!   nested call; measured).
//!
//! Compound literals, statement expressions and value flow through other
//! variables are outside the front-end walk (GCC reports some of them from
//! its `-O2` isolate-paths pass, which works on optimized SSA and is not part
//! of this rule).
//!
//! Code generation follows GCC too: a definite local-variable return is
//! evaluated for its side effects and then returns a null pointer, at every
//! optimization level (gcc-16 -O0 and -O2 both emit `xorl %eax, %eax`), so a
//! later dereference faults deterministically instead of reading a dead
//! frame -- and the null is smaller than the `lea` Clang 23.1.4 and ICX
//! 2026.2 emit. A label address is only diagnosed: its value is not a
//! dangling pointer into a dead frame.

use super::SemanticAnalyzer;
use crate::common::types::{CType, PtrQuals};
use crate::frontend::parser::ast::{BinOp, Expr, PostfixOp, UnaryOp};

const LOCAL_MESSAGE: &str = "function returns address of local variable";
const LABEL_MESSAGE: &str = "function returns address of label";

/// What a returned value's address designates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Escape {
    Local,
    Label,
}

impl SemanticAnalyzer {
    /// Diagnose `return expr;` in a function returning `return_type`, and
    /// record a local-variable return for the lowerer's null substitution.
    pub(super) fn check_return_local_addr(&mut self, expr: &Expr, return_type: &CType) {
        if !matches!(return_type, CType::Pointer(..)) {
            return;
        }
        let Some(escape) = self.returned_escape(expr) else {
            return;
        };
        let message = match escape {
            Escape::Local => {
                self.result.local_address_returns.insert(expr.id());
                LOCAL_MESSAGE
            }
            Escape::Label => LABEL_MESSAGE,
        };
        let span = self.return_local_addr_caret(expr);
        self.diagnostics.borrow_mut().warning_with_kind(
            message,
            span,
            crate::common::error::WarningKind::ReturnLocalAddr,
        );
    }

    /// GCC's location for the returned value (measured): `_Generic` and
    /// `__builtin_choose_expr` are replaced by their selected operand while
    /// parsing, so the caret is that operand's; an array that decays carets
    /// the start of its expression (`return s.a;` at `s`, `return a[1];` at
    /// `a`); everything else carets as an operand does (a binary operator
    /// at the operator, a cast at its `(`, a conditional at its `:`, a comma
    /// expression at its last comma).
    fn return_local_addr_caret(&self, expr: &Expr) -> crate::common::source::Span {
        let mut cursor = expr;
        loop {
            cursor = match cursor {
                Expr::GenericSelection(controlling, associations, _) => {
                    match self
                        .expr_type_checker()
                        .generic_selection_index(controlling, associations)
                    {
                        Some(index) => &associations[index].expr,
                        None => break,
                    }
                }
                Expr::FunctionCall(..) => match self.chosen_operand(cursor) {
                    Some(chosen) => chosen,
                    None => break,
                },
                _ => break,
            };
        }
        let decays = self.is_array_typed(cursor)
            && matches!(
                cursor,
                Expr::Identifier(..)
                    | Expr::MemberAccess(..)
                    | Expr::PointerMemberAccess(..)
                    | Expr::ArraySubscript(..)
                    | Expr::Deref(..)
            );
        if decays {
            self.expr_start_span(cursor)
        } else {
            self.operand_caret_span(cursor)
        }
    }

    /// GCC's value walk: strip conversions and offset arithmetic, apply the
    /// front end's folds, and classify the address at the bottom.
    fn returned_escape(&self, expr: &Expr) -> Option<Escape> {
        let mut cursor = expr;
        loop {
            cursor = match cursor {
                Expr::Cast(..) if self.is_bool_typed(cursor) => return None,
                Expr::Cast(_, inner, _, _) => inner,
                Expr::BinaryOp(BinOp::Add, lhs, rhs, _) => self.addition_operand(lhs, rhs),
                Expr::BinaryOp(BinOp::Sub, lhs, rhs, _) => self.subtraction_operand(lhs, rhs)?,
                Expr::Comma(lhs, rhs, _) if !self.has_side_effects(lhs) => rhs,
                Expr::Conditional(cond, then_e, else_e, _) => match self.condition_truth(cond) {
                    Some(true) => then_e,
                    Some(false) => else_e,
                    None if !self.has_side_effects(cond) && same_expr(then_e, else_e) => then_e,
                    None => return None,
                },
                Expr::GnuConditional(cond, else_e, _) => match self.condition_truth(cond)? {
                    true => cond,
                    false => else_e,
                },
                Expr::GenericSelection(controlling, associations, _) => {
                    let index = self
                        .expr_type_checker()
                        .generic_selection_index(controlling, associations)?;
                    &associations[index].expr
                }
                Expr::FunctionCall(..) => self.chosen_operand(cursor)?,
                Expr::AddressOf(inner, _) => {
                    return self.address_root_is_local(inner).then_some(Escape::Local);
                }
                Expr::LabelAddr(..) => return Some(Escape::Label),
                Expr::Identifier(..)
                | Expr::MemberAccess(..)
                | Expr::PointerMemberAccess(..)
                | Expr::ArraySubscript(..)
                | Expr::Deref(..)
                    if self.is_array_typed(cursor) =>
                {
                    // An array operand decays: an implicit `&`.
                    return self.address_root_is_local(cursor).then_some(Escape::Local);
                }
                _ => return None,
            };
        }
    }

    /// The operand GCC's walk continues with for `lhs + rhs`.
    fn addition_operand<'e>(&self, lhs: &'e Expr, rhs: &'e Expr) -> &'e Expr {
        // Pointer arithmetic: GCC's POINTER_PLUS_EXPR always has the pointer
        // as operand 0, whichever side it was written on.
        if self.is_pointer_like(lhs) {
            return lhs;
        }
        if self.is_pointer_like(rhs) {
            return rhs;
        }
        // Integer `+` is commutative and canonicalized by
        // `tree_swap_operands_p`: a literal constant goes second, then an
        // address-invariant (TREE_CONSTANT) operand goes second.
        if self.eval_const_expr_value(rhs).is_some() {
            lhs
        } else if self.eval_const_expr_value(lhs).is_some() {
            rhs
        } else if self.is_address_invariant(rhs) {
            lhs
        } else if self.is_address_invariant(lhs) {
            rhs
        } else {
            lhs
        }
    }

    /// The operand GCC's walk continues with for `lhs - rhs`, if any.
    fn subtraction_operand<'e>(&self, lhs: &'e Expr, rhs: &'e Expr) -> Option<&'e Expr> {
        if self.is_pointer_like(lhs) {
            if !self.is_pointer_like(rhs) {
                // `p - n` is POINTER_PLUS_EXPR (p, -n).
                return Some(lhs);
            }
            // A pointer difference is not followed, except `p - (T *)0`,
            // which folds to the conversion of `p`.
            return self
                .expr_type_checker()
                .is_null_pointer_constant(strip_casts(rhs))
                .then_some(lhs);
        }
        // Integer `-`: a right operand that is, or converts from, a pointer
        // "may be valid" (GCC's comment) and stops the walk.
        let mut operand = rhs;
        loop {
            if self.is_pointer_like(operand) {
                return None;
            }
            match operand {
                Expr::Cast(..) if self.is_bool_typed(operand) => break,
                Expr::Cast(_, inner, _, _) => operand = inner,
                _ => break,
            }
        }
        Some(lhs)
    }

    /// Whether the address of the object `lvalue` designates belongs to an
    /// automatic object (or parameter) of the function being analyzed.
    fn address_root_is_local(&self, lvalue: &Expr) -> bool {
        let mut cursor = lvalue;
        loop {
            cursor = match cursor {
                Expr::Identifier(name, _) => return self.is_current_function_automatic(name),
                Expr::MemberAccess(base, _, _) => base,
                Expr::UnaryOp(UnaryOp::RealPart | UnaryOp::ImagPart, inner, _) => inner,
                Expr::ArraySubscript(lhs, rhs, _) => {
                    // ARRAY_REF descends to an array object; a subscript
                    // through a pointer is an INDIRECT_REF and stops.
                    if self.is_array_typed(lhs) {
                        lhs
                    } else if self.is_array_typed(rhs) {
                        rhs
                    } else {
                        return false;
                    }
                }
                // `*&e` and `(&e)->m` fold to `e` and `e.m`; `*a` on an
                // array is its first element.
                Expr::Deref(inner, _) | Expr::PointerMemberAccess(inner, _, _) => {
                    // Only a cast-free `*&e` folds: `*(T *)&e` is a
                    // different object type.
                    match inner.as_ref() {
                        Expr::AddressOf(object, _) => object,
                        _ if self.is_array_typed(inner) => inner,
                        _ => return false,
                    }
                }
                _ => return false,
            };
        }
    }

    /// Whether `name` is an automatic object or parameter declared inside
    /// the function currently analyzed (not an enclosing one).
    fn is_current_function_automatic(&self, name: &str) -> bool {
        self.symbol_table
            .automatic_object_scope(name)
            .is_some_and(|scope| scope >= self.function_scope_floor)
    }

    /// The truth of a condition when the front end folds it: integer and
    /// floating constants, and addresses that cannot be null.
    fn condition_truth(&self, cond: &Expr) -> Option<bool> {
        if let Some(value) = self.eval_const_expr_value(cond) {
            return Some(!value.is_zero());
        }
        self.address_is_nonnull(strip_casts(cond)).then_some(true)
    }

    /// An address GCC proves non-null at parse time: a string literal, an
    /// array that decays, `&object`, or a function designator, whose root is
    /// not weak.
    fn address_is_nonnull(&self, expr: &Expr) -> bool {
        let root: &Expr = match expr {
            Expr::StringLiteral(..)
            | Expr::WideStringLiteral(..)
            | Expr::Char16StringLiteral(..) => return true,
            Expr::AddressOf(object, _) => object,
            Expr::Identifier(..) if self.is_array_typed(expr) || self.is_function_typed(expr) => {
                expr
            }
            Expr::MemberAccess(..) | Expr::ArraySubscript(..) if self.is_array_typed(expr) => expr,
            _ => return false,
        };
        let mut cursor = root;
        loop {
            cursor = match cursor {
                Expr::Identifier(name, _) => {
                    return self.symbol_table.lookup(name).is_some()
                        && !self.symbol_table.is_enumerator(name)
                        && !self.weak_symbols.contains(name.as_str());
                }
                Expr::MemberAccess(base, _, _) => base,
                Expr::ArraySubscript(lhs, _, _) if self.is_array_typed(lhs) => lhs,
                Expr::ArraySubscript(_, rhs, _) if self.is_array_typed(rhs) => rhs,
                Expr::StringLiteral(..) => return true,
                _ => return false,
            };
        }
    }

    /// GCC's TREE_CONSTANT for an integer operand: the (converted) address
    /// of a declared object at a constant offset, or such a value combined
    /// with constants.
    fn is_address_invariant(&self, expr: &Expr) -> bool {
        match strip_casts(expr) {
            Expr::AddressOf(object, _) => self.is_invariant_lvalue(object),
            e @ (Expr::Identifier(..) | Expr::MemberAccess(..) | Expr::ArraySubscript(..))
                if self.is_array_typed(e) =>
            {
                self.is_invariant_lvalue(e)
            }
            Expr::StringLiteral(..) => true,
            Expr::BinaryOp(BinOp::Add | BinOp::Sub | BinOp::Mul, lhs, rhs, _) => {
                (self.is_address_invariant(lhs) || self.eval_const_expr_value(lhs).is_some())
                    && (self.is_address_invariant(rhs) || self.eval_const_expr_value(rhs).is_some())
                    && !(self.eval_const_expr_value(lhs).is_some()
                        && self.eval_const_expr_value(rhs).is_some())
            }
            _ => false,
        }
    }

    /// An lvalue whose address is invariant within the function: a declared
    /// object or string literal, through members and constant subscripts.
    fn is_invariant_lvalue(&self, lvalue: &Expr) -> bool {
        match lvalue {
            Expr::Identifier(name, _) => {
                self.symbol_table.lookup(name).is_some() && !self.symbol_table.is_enumerator(name)
            }
            Expr::StringLiteral(..) | Expr::CompoundLiteral(..) => true,
            Expr::MemberAccess(base, _, _) => self.is_invariant_lvalue(base),
            Expr::UnaryOp(UnaryOp::RealPart | UnaryOp::ImagPart, inner, _) => {
                self.is_invariant_lvalue(inner)
            }
            Expr::ArraySubscript(lhs, rhs, _) => {
                let (array, index) = if self.is_array_typed(lhs) {
                    (lhs, rhs)
                } else {
                    (rhs, lhs)
                };
                self.is_array_typed(array)
                    && self.eval_const_expr_value(index).is_some()
                    && self.is_invariant_lvalue(array)
            }
            _ => false,
        }
    }

    /// The operand `__builtin_choose_expr` selects; `None` for any other
    /// call.
    fn chosen_operand<'e>(&self, call: &'e Expr) -> Option<&'e Expr> {
        let Expr::FunctionCall(callee, args, _) = call else {
            return None;
        };
        let Expr::Identifier(name, _) = callee.as_ref() else {
            return None;
        };
        if name != "__builtin_choose_expr" || args.len() < 3 {
            return None;
        }
        if !self.is_integer_constant_expr(&args[0]) {
            return None;
        }
        let first = self.expr_type_checker().choose_expr_takes_first(&args[0]);
        Some(if first { &args[1] } else { &args[2] })
    }

    /// GCC's TREE_SIDE_EFFECTS, conservatively: any call, assignment,
    /// increment, statement expression, `va_arg`, or read of an object not
    /// known to be non-volatile.
    fn has_side_effects(&self, expr: &Expr) -> bool {
        match expr {
            Expr::FunctionCall(..)
            | Expr::Assign(..)
            | Expr::CompoundAssign(..)
            | Expr::PostfixOp(PostfixOp::PostInc | PostfixOp::PostDec, ..)
            | Expr::UnaryOp(UnaryOp::PreInc | UnaryOp::PreDec, ..)
            | Expr::StmtExpr(..)
            | Expr::VaArg(..) => true,
            Expr::Identifier(name, _) => {
                // Enumerators and functions are not objects.
                if self.symbol_table.is_enumerator(name) || self.is_function_typed(expr) {
                    return false;
                }
                self.object_may_be_volatile(expr)
            }
            Expr::MemberAccess(base, _, _) => {
                self.has_side_effects(base) || self.object_may_be_volatile(expr)
            }
            Expr::PointerMemberAccess(base, _, _) | Expr::Deref(base, _) => {
                self.has_side_effects(base) || self.object_may_be_volatile(expr)
            }
            Expr::ArraySubscript(lhs, rhs, _) => {
                self.has_side_effects(lhs)
                    || self.has_side_effects(rhs)
                    || self.object_may_be_volatile(expr)
            }
            Expr::BinaryOp(_, lhs, rhs, _) | Expr::Comma(lhs, rhs, _) => {
                self.has_side_effects(lhs) || self.has_side_effects(rhs)
            }
            Expr::UnaryOp(_, inner, _) | Expr::Cast(_, inner, _, _) => self.has_side_effects(inner),
            Expr::AddressOf(inner, _) => self.address_operand_has_side_effects(inner),
            Expr::Conditional(cond, then_e, else_e, _) => {
                self.has_side_effects(cond)
                    || self.has_side_effects(then_e)
                    || self.has_side_effects(else_e)
            }
            Expr::GnuConditional(cond, else_e, _) => {
                self.has_side_effects(cond) || self.has_side_effects(else_e)
            }
            Expr::IntLiteral(..)
            | Expr::UIntLiteral(..)
            | Expr::LongLiteral(..)
            | Expr::ULongLiteral(..)
            | Expr::LongLongLiteral(..)
            | Expr::ULongLongLiteral(..)
            | Expr::FloatLiteral(..)
            | Expr::FloatLiteralF32(..)
            | Expr::CharLiteral(..)
            | Expr::StringLiteral(..)
            | Expr::WideStringLiteral(..)
            | Expr::Char16StringLiteral(..)
            | Expr::Sizeof(..)
            | Expr::Alignof(..)
            | Expr::GnuAlignof(..)
            | Expr::LabelAddr(..) => false,
            _ => true,
        }
    }

    /// `&lvalue` reads nothing: only the subscripts and pointer operands on
    /// the way can have side effects.
    fn address_operand_has_side_effects(&self, lvalue: &Expr) -> bool {
        match lvalue {
            Expr::Identifier(..) | Expr::StringLiteral(..) => false,
            Expr::MemberAccess(base, _, _) => self.address_operand_has_side_effects(base),
            Expr::ArraySubscript(lhs, rhs, _) => {
                let (array, index) = if self.is_array_typed(lhs) {
                    (lhs, rhs)
                } else {
                    (rhs, lhs)
                };
                self.has_side_effects(index)
                    || if self.is_array_typed(array) {
                        self.address_operand_has_side_effects(array)
                    } else {
                        self.has_side_effects(array)
                    }
            }
            Expr::Deref(inner, _) | Expr::PointerMemberAccess(inner, _, _) => {
                self.has_side_effects(inner)
            }
            _ => true,
        }
    }

    /// Whether reading the object `lvalue` designates may be a volatile
    /// access: true unless its qualifiers are recorded without `volatile`.
    fn object_may_be_volatile(&self, lvalue: &Expr) -> bool {
        // An array or function operand is not read: it decays to an address.
        if self.is_array_typed(lvalue) || self.is_function_typed(lvalue) {
            return false;
        }
        self.expr_type_checker()
            .lvalue_object_cvr(lvalue)
            .is_none_or(|cvr| cvr & PtrQuals::VOLATILE != 0)
    }

    fn inferred_ctype(&self, expr: &Expr) -> Option<CType> {
        self.expr_type_checker().infer_expr_ctype(expr)
    }

    fn is_array_typed(&self, expr: &Expr) -> bool {
        matches!(self.inferred_ctype(expr), Some(CType::Array(..)))
    }

    fn is_function_typed(&self, expr: &Expr) -> bool {
        matches!(self.inferred_ctype(expr), Some(CType::Function(..)))
    }

    fn is_bool_typed(&self, expr: &Expr) -> bool {
        matches!(self.inferred_ctype(expr), Some(CType::Bool))
    }

    /// A pointer, or an array or function operand that decays to one.
    fn is_pointer_like(&self, expr: &Expr) -> bool {
        matches!(
            self.inferred_ctype(expr),
            Some(CType::Pointer(..) | CType::Array(..) | CType::Function(..))
        )
    }
}

/// Strip explicit casts (GCC's conversions; the caller handles `_Bool`).
fn strip_casts(mut expr: &Expr) -> &Expr {
    while let Expr::Cast(_, inner, _, _) = expr {
        expr = inner;
    }
    expr
}

/// Structural equality of two side-effect-free expressions, as GCC's
/// `operand_equal_p` decides it for the shapes a returned address takes.
fn same_expr(a: &Expr, b: &Expr) -> bool {
    match (a, b) {
        (Expr::Identifier(x, _), Expr::Identifier(y, _)) => x == y,
        (Expr::AddressOf(x, _), Expr::AddressOf(y, _)) | (Expr::Deref(x, _), Expr::Deref(y, _)) => {
            same_expr(x, y)
        }
        (Expr::MemberAccess(x, f, _), Expr::MemberAccess(y, g, _))
        | (Expr::PointerMemberAccess(x, f, _), Expr::PointerMemberAccess(y, g, _)) => {
            f == g && same_expr(x, y)
        }
        (Expr::ArraySubscript(x0, x1, _), Expr::ArraySubscript(y0, y1, _))
        | (Expr::BinaryOp(_, x0, x1, _), Expr::BinaryOp(_, y0, y1, _))
            if same_binop(a, b) =>
        {
            same_expr(x0, y0) && same_expr(x1, y1)
        }
        (Expr::IntLiteral(x, _), Expr::IntLiteral(y, _)) => x == y,
        (Expr::LabelAddr(x, _), Expr::LabelAddr(y, _)) => x == y,
        _ => false,
    }
}

fn same_binop(a: &Expr, b: &Expr) -> bool {
    match (a, b) {
        (Expr::BinaryOp(x, ..), Expr::BinaryOp(y, ..)) => x == y,
        (Expr::ArraySubscript(..), Expr::ArraySubscript(..)) => true,
        _ => false,
    }
}
