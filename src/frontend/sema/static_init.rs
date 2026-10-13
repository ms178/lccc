//! Constant initializers of block-scope `static` objects (C11 6.7.9p4).
//!
//! A `static` object declared in a block must be initialized with a constant
//! expression, exactly like a file-scope object. The two forms that go wrong
//! in practice are the address of an automatic object and the value of a
//! non-constant object:
//!
//! ```c
//! void f(int n) { int a; static int *p = &a; static int *q = &n; }
//! ```
//!
//! lccc used to accept both, emitting a store of a stack address into static
//! storage that dangles as soon as `f` returns. GCC 16.2 rejects them with
//! "initializer element is not constant".
//!
//! The check is a three-valued evaluation of each initializer leaf:
//!
//! * `Value`: an arithmetic constant (literals, enumerators, `sizeof`,
//!   folded integer arithmetic, the value of a const object whose initializer
//!   was constant, `?:` on a constant condition).
//! * `Address(root)`: an address constant. `root` names the static object
//!   the address is into, so `&a.x - &a.y`-style differences can be checked
//!   for a common object; `None` for string literals and function designators.
//! * `Bad`: anything else, including the address of an automatic object, the
//!   value of a non-const object, calls, assignments, comma, and compound
//!   literals (which are automatic in block scope).
//!
//! The rules were measured against GCC 16.2 (see
//! `tests/regression/static_local/`), including the GNU folding of const
//! objects and the unevaluated operand of `?:`.

use crate::common::source::Span;
use crate::common::symbol_table::SymbolTable;
use crate::common::types::{AddressSpace, CType};
use crate::frontend::parser::ast::{BinOp, Expr, GenericAssociation, Initializer, UnaryOp};

/// What an initializer expression is, for the purpose of a static initializer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind<'a> {
    Value,
    Address(Option<&'a str>),
    Bad,
}

/// Evaluates initializer expressions against the sema state. The closures
/// keep this module free of the analyzer's borrow structure: `eval` is the
/// integer constant evaluator and `ctype` the expression type inference.
pub(super) struct StaticInitChecker<'s> {
    symbols: &'s SymbolTable,
    eval: &'s dyn Fn(&Expr) -> Option<i64>,
    ctype: &'s dyn Fn(&Expr) -> Option<CType>,
    /// The `_Generic` selection rule: the index of the selected association.
    select: &'s dyn Fn(&Expr, &[GenericAssociation]) -> Option<usize>,
}

impl<'s> StaticInitChecker<'s> {
    pub(super) fn new(
        symbols: &'s SymbolTable,
        eval: &'s dyn Fn(&Expr) -> Option<i64>,
        ctype: &'s dyn Fn(&Expr) -> Option<CType>,
        select: &'s dyn Fn(&Expr, &[GenericAssociation]) -> Option<usize>,
    ) -> Self {
        Self {
            symbols,
            eval,
            ctype,
            select,
        }
    }

    /// The spans of every initializer leaf that is not a constant. Designator
    /// expressions are not checked here: they are constant by a separate rule.
    pub(super) fn non_constant_leaves(&self, init: &Initializer) -> Vec<Span> {
        let mut out = Vec::new();
        self.collect_bad(init, &mut out);
        out
    }

    /// Whether `init` is a constant initializer, so an object it initializes
    /// may have its value folded (see `SymbolTable::mark_folded`).
    pub(super) fn is_constant_initializer(&self, init: &Initializer) -> bool {
        self.non_constant_leaves(init).is_empty()
    }

    fn collect_bad(&self, init: &Initializer, out: &mut Vec<Span>) {
        match init {
            Initializer::Expr(e) => {
                if self.value(e) == Kind::Bad {
                    out.push(e.span());
                }
            }
            Initializer::List(items) => {
                for item in items {
                    self.collect_bad(&item.init, out);
                }
            }
        }
    }

    /// The kind of an expression used as a value (after array and function
    /// designators decay).
    pub(super) fn value<'e>(&self, e: &'e Expr) -> Kind<'e> {
        if (self.eval)(e).is_some() {
            return Kind::Value;
        }
        match e {
            Expr::IntLiteral(..)
            | Expr::UIntLiteral(..)
            | Expr::LongLiteral(..)
            | Expr::ULongLiteral(..)
            | Expr::LongLongLiteral(..)
            | Expr::ULongLongLiteral(..)
            | Expr::FloatLiteral(..)
            | Expr::FloatLiteralF32(..)
            | Expr::FloatLiteralLongDouble(..)
            | Expr::FloatLiteralF128(..)
            | Expr::FloatLiteralDecimal(..)
            | Expr::ImaginaryLiteral(..)
            | Expr::ImaginaryLiteralF32(..)
            | Expr::ImaginaryLiteralLongDouble(..)
            | Expr::CharLiteral(..) => Kind::Value,
            Expr::StringLiteral(..)
            | Expr::WideStringLiteral(..)
            | Expr::Char16StringLiteral(..) => Kind::Address(None),
            Expr::LabelAddr(..) => Kind::Address(None),
            Expr::Identifier(name, _) => self.identifier_value(name),
            Expr::AddressOf(inner, _) => self.address(inner),
            Expr::Cast(_, inner, _, _) => {
                let kind = self.value(inner);
                // An address constant converts only to a pointer, or to an
                // integer as wide as a pointer (`(long)&a`). `(int)&a` and
                // `(double)&a` are not constants (GCC 16.2 rejects both).
                match (kind, (self.ctype)(e)) {
                    (Kind::Address(_), Some(target)) if !address_cast_is_constant(&target) => {
                        Kind::Bad
                    }
                    _ => kind,
                }
            }
            Expr::UnaryOp(op, inner, _) => match op {
                UnaryOp::Plus | UnaryOp::Neg | UnaryOp::BitNot => {
                    if self.value(inner) == Kind::Value {
                        Kind::Value
                    } else {
                        Kind::Bad
                    }
                }
                // `!&g` is 0: an address is never null.
                UnaryOp::LogicalNot => match self.value(inner) {
                    Kind::Value | Kind::Address(Some(_)) => Kind::Value,
                    _ => Kind::Bad,
                },
                _ => Kind::Bad,
            },
            Expr::BinaryOp(op, lhs, rhs, _) => self.binary(*op, lhs, rhs),
            Expr::Conditional(cond, then_e, else_e, _) => match (self.eval)(cond) {
                // Only the selected operand is evaluated, so the other one may
                // name an automatic object (GCC accepts `0 ? &a : 0`).
                Some(0) => self.value(else_e),
                Some(_) => self.value(then_e),
                None => Kind::Bad,
            },
            // `c ?: f` yields `c` when it is nonzero, which is a constant here.
            Expr::GnuConditional(cond, else_e, _) => match (self.eval)(cond) {
                Some(0) => self.value(else_e),
                Some(_) => Kind::Value,
                None => Kind::Bad,
            },
            Expr::FunctionCall(callee, args, _) if self.folds_to_constant(callee, args) => {
                Kind::Value
            }
            // Only the selected association is part of the initializer. The
            // others are never evaluated, so they may name automatic objects
            // (GCC 16.2 accepts `_Generic(n, int: (int *)0, default: (int *)&a)`).
            Expr::GenericSelection(controlling, associations, _) => {
                match (self.select)(controlling, associations).and_then(|i| associations.get(i)) {
                    Some(assoc) => self.value(&assoc.expr),
                    None => Kind::Bad,
                }
            }
            // A file-scope compound literal has static storage: an address constant.
            Expr::CompoundLiteral(..) => self.compound_literal_kind(),
            Expr::Sizeof(..)
            | Expr::Alignof(..)
            | Expr::AlignofExpr(..)
            | Expr::GnuAlignof(..)
            | Expr::GnuAlignofExpr(..) => Kind::Value,
            // A character of an ASCII string literal is a constant (`"abc"[1]`).
            // `*"abc"` is not (GCC rejects it).
            Expr::ArraySubscript(base, index, _)
                if matches!(&**base, Expr::StringLiteral(text, _) if text.is_ascii())
                    && self.value(index) == Kind::Value =>
            {
                Kind::Value
            }
            // An array-typed member or element decays to the address of its
            // storage (`gobj.a + 1`, `g_char_2d[1]`): an address constant when
            // the object has static storage, as `identifier_value` decides for
            // a plain array name.
            Expr::MemberAccess(..) | Expr::PointerMemberAccess(..) | Expr::ArraySubscript(..)
                if matches!((self.ctype)(e), Some(CType::Array(..))) =>
            {
                self.address(e)
            }
            Expr::MemberAccess(..) | Expr::ArraySubscript(..) => {
                if self.folded_object_read(e) {
                    Kind::Value
                } else {
                    Kind::Bad
                }
            }
            _ => Kind::Bad,
        }
    }

    /// Whether a call is folded to a constant by GCC: a pure math or bit
    /// builtin (or the libm function it stands for) whose arguments are all
    /// constant. `INFINITY`, `NAN`, `sqrt(4.0)`, `__builtin_isnan(1.0)` are
    /// accepted by GCC 16.2 in static initializers; `__builtin_isnan(x)` for a
    /// non-constant `x` is not. Calls outside the list are not folded.
    fn folds_to_constant(&self, callee: &Expr, args: &[Expr]) -> bool {
        let Expr::Identifier(name, _) = callee else {
            return false;
        };
        if !is_foldable_builtin(name) {
            return false;
        }
        args.iter().all(|a| self.value(a) != Kind::Bad)
    }

    /// The kind of an expression used as the operand of `&` (an lvalue whose
    /// address is taken). The result is `Bad` for any object without static
    /// storage.
    fn address<'e>(&self, e: &'e Expr) -> Kind<'e> {
        match e {
            Expr::Identifier(name, _) => match self.symbols.lookup(name) {
                None => Kind::Address(None),
                Some(sym) if matches!(sym.ty, CType::Function(_)) => Kind::Address(None),
                Some(_) if self.symbols.is_automatic_object(name) => Kind::Bad,
                Some(_) => Kind::Address(Some(name.as_str())),
            },
            Expr::StringLiteral(..) | Expr::WideStringLiteral(..) => Kind::Address(None),
            Expr::MemberAccess(base, _, _) => self.address(base),
            Expr::PointerMemberAccess(base, _, _) => match self.value(base) {
                Kind::Bad => Kind::Bad,
                Kind::Address(root) => Kind::Address(root),
                Kind::Value => Kind::Address(None),
            },
            Expr::ArraySubscript(base, index, _) => {
                if self.value(index) != Kind::Value {
                    return Kind::Bad;
                }
                let base_is_array = matches!((self.ctype)(base), Some(CType::Array(..)));
                if base_is_array {
                    self.address(base)
                } else {
                    match self.value(base) {
                        Kind::Bad => Kind::Bad,
                        Kind::Address(root) => Kind::Address(root),
                        Kind::Value => Kind::Address(None),
                    }
                }
            }
            Expr::Deref(inner, _) => match self.value(inner) {
                Kind::Bad => Kind::Bad,
                Kind::Address(root) => Kind::Address(root),
                Kind::Value => Kind::Address(None),
            },
            Expr::CompoundLiteral(..) => self.compound_literal_kind(),
            _ => Kind::Bad,
        }
    }

    /// A compound literal has static storage at file scope (an address
    /// constant). In block scope it is automatic, so it is not constant.
    fn compound_literal_kind<'e>(&self) -> Kind<'e> {
        if self.symbols.at_file_scope() {
            Kind::Address(None)
        } else {
            Kind::Bad
        }
    }

    /// An identifier used as a value: an array or function decays to its
    /// address, an object contributes its value only if it is a folded const.
    fn identifier_value<'e>(&self, name: &'e str) -> Kind<'e> {
        match self.symbols.lookup(name) {
            // Undeclared: already diagnosed; do not report a second error.
            None => Kind::Value,
            Some(sym) => match &sym.ty {
                CType::Function(_) => Kind::Address(None),
                CType::Array(..) => {
                    if self.symbols.is_automatic_object(name) {
                        Kind::Bad
                    } else {
                        Kind::Address(Some(name))
                    }
                }
                _ if self.symbols.is_folded(name) => match sym.ty {
                    // A folded const pointer carries an address constant.
                    CType::Pointer(..) => Kind::Address(None),
                    _ => Kind::Value,
                },
                _ => Kind::Bad,
            },
        }
    }

    /// Whether `e` reads an element or member of a folded const object.
    fn folded_object_read(&self, e: &Expr) -> bool {
        match e {
            Expr::Identifier(name, _) => self.symbols.is_folded(name),
            Expr::MemberAccess(base, _, _) => self.folded_object_read(base),
            Expr::ArraySubscript(base, index, _) => {
                matches!((self.ctype)(base), Some(CType::Array(..)))
                    && self.value(index) == Kind::Value
                    && self.folded_object_read(base)
            }
            _ => false,
        }
    }

    /// A relational or equality comparison. GCC folds these for address
    /// constants with the rules the lowering implements (`eval_global_relational`):
    /// two addresses into one object compare by offset, distinct objects are
    /// unequal, and an address against a null constant is decided. Ordering
    /// across distinct objects is rejected by GCC, and an address of unknown
    /// identity (a function or string literal) is not compared here.
    fn comparison<'e>(
        &self,
        op: BinOp,
        lhs: &'e Expr,
        rhs: &'e Expr,
        a: Kind<'e>,
        b: Kind<'e>,
    ) -> Kind<'e> {
        let is_null = |e: &Expr| (self.eval)(e) == Some(0);
        // GCC 16.2 does not fold the ordering of an address converted to an
        // integer against the null constant: `(long)&a > 0` is "not computable
        // at load time". The pointer form `&a > 0` is folded.
        let integer_address = |e: &Expr| matches!((self.ctype)(e), Some(t) if t.is_integer());
        let ordering = !matches!(op, BinOp::Eq | BinOp::Ne);
        match (a, b) {
            (Kind::Value, Kind::Value) => Kind::Value,
            (Kind::Address(Some(x)), Kind::Address(Some(y))) => {
                if x == y || !ordering {
                    Kind::Value
                } else {
                    Kind::Bad
                }
            }
            (Kind::Address(Some(_)), Kind::Value)
                if is_null(rhs) && !(ordering && integer_address(lhs)) =>
            {
                Kind::Value
            }
            (Kind::Value, Kind::Address(Some(_)))
                if is_null(lhs) && !(ordering && integer_address(rhs)) =>
            {
                Kind::Value
            }
            _ => Kind::Bad,
        }
    }

    fn binary<'e>(&self, op: BinOp, lhs: &'e Expr, rhs: &'e Expr) -> Kind<'e> {
        let (a, b) = (self.value(lhs), self.value(rhs));
        match op {
            BinOp::Add => match (a, b) {
                (Kind::Value, Kind::Value) => Kind::Value,
                (Kind::Address(root), Kind::Value) | (Kind::Value, Kind::Address(root)) => {
                    Kind::Address(root)
                }
                _ => Kind::Bad,
            },
            BinOp::Sub => match (a, b) {
                (Kind::Value, Kind::Value) => Kind::Value,
                (Kind::Address(root), Kind::Value) => Kind::Address(root),
                // The difference of two addresses in one object is a constant.
                (Kind::Address(Some(x)), Kind::Address(Some(y))) if x == y => Kind::Value,
                // The difference of two label addresses is an assembler-time
                // constant (GCC accepts `static int o[] = {&&a - &&b}`).
                _ if is_label_address(lhs) && is_label_address(rhs) => Kind::Value,
                _ => Kind::Bad,
            },
            BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                self.comparison(op, lhs, rhs, a, b)
            }
            // `&g && 1` is 1: an address is true. Only named addresses qualify.
            BinOp::LogicalAnd | BinOp::LogicalOr => {
                let truthy = |k: Kind| matches!(k, Kind::Value | Kind::Address(Some(_)));
                if truthy(a) && truthy(b) {
                    Kind::Value
                } else {
                    Kind::Bad
                }
            }
            _ => {
                if a == Kind::Value && b == Kind::Value {
                    Kind::Value
                } else {
                    Kind::Bad
                }
            }
        }
    }
}

/// Whether an address constant may be converted to `target`: a pointer, or an
/// integer exactly as wide as a pointer. A `_Bool`, a narrower integer, or a
/// floating type is not a constant of that conversion.
fn address_cast_is_constant(target: &CType) -> bool {
    match target {
        CType::Pointer(..) => true,
        t if t.is_integer() => {
            let pointer = CType::Pointer(Box::new(CType::Void), AddressSpace::Default);
            t.size() == pointer.size()
        }
        _ => false,
    }
}

/// Whether `e` is a label address `&&label`, possibly under casts.
fn is_label_address(e: &Expr) -> bool {
    match e {
        Expr::LabelAddr(..) => true,
        Expr::Cast(_, inner, _, _) => is_label_address(inner),
        _ => false,
    }
}

/// The builtins GCC folds in constant expressions, by name stem. Each stem is
/// accepted with the `__builtin_` prefix and one of `FOLDABLE_SUFFIXES`.
const FOLDABLE_BUILTIN_STEMS: &[&str] = &[
    "inf",
    "nan",
    "nans",
    "huge_val",
    "fabs",
    "copysign",
    "sqrt",
    "floor",
    "ceil",
    "trunc",
    "round",
    "fmax",
    "fmin",
    "fma",
    "isnan",
    "isinf",
    "isfinite",
    "isnormal",
    "signbit",
    "fpclassify",
    "clz",
    "ctz",
    "popcount",
    "parity",
    "ffs",
    "bswap",
    "expect",
    "constant_p",
];

/// Precision and width suffixes of the stems above (`__builtin_nanf`,
/// `__builtin_clzll`, `__builtin_bswap32`).
const FOLDABLE_SUFFIXES: &[&str] = &["", "f", "l", "ll", "s", "16", "32", "64"];

/// Whether `name` is a builtin, or the libm function of the same stem, that GCC
/// folds in a constant expression. A user function that shadows one of these
/// names is treated as the builtin. That can only accept more than GCC does,
/// never reject a valid program.
fn is_foldable_builtin(name: &str) -> bool {
    if let Some(stem) = name.strip_prefix("__builtin_") {
        return FOLDABLE_BUILTIN_STEMS.iter().any(|base| {
            stem.strip_prefix(base)
                .is_some_and(|suffix| FOLDABLE_SUFFIXES.contains(&suffix))
        });
    }
    let base = name.trim_end_matches(['f', 'l']);
    matches!(
        base,
        "sqrt"
            | "fabs"
            | "floor"
            | "ceil"
            | "trunc"
            | "round"
            | "fmax"
            | "fmin"
            | "fma"
            | "copysign"
            | "nan"
            | "inf"
            | "huge_val"
    )
}
