//! Expression-level CType inference for semantic analysis.
//!
//! This module provides CType inference for expressions using only sema-available
//! state: SymbolTable (variable/function types), TypeContext (typedefs, struct
//! layouts, enum constants), and FunctionInfo (function signatures).
//!
//! This is a key step toward making sema produce a typed AST: by inferring
//! expression types in sema rather than the lowerer, we can:
//! - Report type errors as diagnostics instead of lowering panics
//! - Enable future type annotations on AST nodes
//! - Reduce the lowerer's responsibility from type-checking + IR emission to
//!   just IR emission
//!
//! The `ExprTypeChecker` operates on immutable references and does not modify
//! any state. It is designed to be called from `SemanticAnalyzer::analyze_expr`.

use super::analysis::FunctionInfo;
use super::type_context::TypeContext;
use crate::common::fx_hash::FxHashMap;
use crate::common::qualifiers;
use crate::common::symbol_table::SymbolTable;
use crate::common::types::{AddressSpace, CType, FunctionType, PtrQuals};
use crate::frontend::parser::ast::{
    BinOp, BlockItem, CastQualifiers, CompoundStmt, DerivedDeclarator, Expr, ExprId,
    GenericAssociation, Stmt, StructFieldDecl, TypeSpecifier, UnaryOp,
};

/// Expression type checker that infers CTypes using only sema-available state.
///
/// This struct borrows the sema state needed for type inference and provides
/// methods to compute the CType of any expression. Unlike the lowerer's
/// `get_expr_ctype`, this does not depend on IR-level state (allocas, IR types,
/// global variable metadata).
///
/// When `expr_types` is provided, previously-computed types are looked up in O(1)
/// instead of re-traversing the AST. This prevents exponential blowup on deeply
/// nested expressions like `1+1+1+...+1` where `infer_binop_ctype` would
/// otherwise recursively re-evaluate lhs/rhs types multiple times per node.
pub struct ExprTypeChecker<'a> {
    /// Symbol table for variable/function type lookup.
    pub symbols: &'a SymbolTable,
    /// Type context for typedef, enum, and struct layout resolution.
    pub types: &'a TypeContext,
    /// Function signatures for return type resolution.
    pub functions: &'a FxHashMap<String, FunctionInfo>,
    /// Pre-computed expression types from bottom-up sema walk (memoization cache).
    /// When set, `infer_expr_ctype` checks this map before recursing.
    pub expr_types: Option<&'a FxHashMap<ExprId, CType>>,
    /// Whether hosted-library builtin aliases are disabled for constant queries.
    pub builtin_policy: crate::common::pass_policy::BuiltinFlags,
    /// The TU is C90/C94: an overflowing signed left shift is still an
    /// integer constant expression (see `ice.rs`).
    pub pre_c99: bool,
}

impl qualifiers::ConstLookup for ExprTypeChecker<'_> {
    fn object_const_base(&self, name: &str) -> bool {
        self.symbols.is_const_base(name)
    }

    fn typedef_const_base(&self, name: &str) -> bool {
        self.types.const_base_typedefs.contains(name)
    }

    fn typedef_const_pointer_level(&self, name: &str) -> Option<usize> {
        self.types
            .typedefs
            .get(name)
            .and_then(qualifiers::typedef_const_pointer_level)
    }

    fn is_pointer(&self, expr: &Expr) -> bool {
        matches!(
            self.infer_expr_ctype(expr),
            Some(CType::Pointer(..) | CType::Array(..))
        )
    }

    fn expr_const_pointer_level(&self, expr: &Expr) -> Option<usize> {
        self.infer_expr_ctype(expr)
            .as_ref()
            .and_then(qualifiers::typedef_const_pointer_level)
    }

    fn type_const_pointer_level(&self, ts: &TypeSpecifier) -> Option<usize> {
        qualifiers::typedef_const_pointer_level(&self.resolve_type_spec(ts))
    }

    fn select_generic<'b>(
        &self,
        controlling: &Expr,
        associations: &'b [GenericAssociation],
    ) -> Option<&'b Expr> {
        self.generic_selection_index(controlling, associations)
            .map(|i| &associations[i].expr)
    }

    fn member_const_base(&self, base: &Expr, field: &str, indirect: bool) -> bool {
        let base_ct = if indirect {
            match self.infer_expr_ctype(base) {
                Some(CType::Pointer(inner, _) | CType::Array(inner, _)) => Some(*inner),
                _ => None,
            }
        } else {
            self.infer_expr_ctype(base)
        };
        let Some(CType::Struct(key) | CType::Union(key)) = base_ct else {
            return false;
        };
        let layouts = self.types.borrow_struct_layouts();
        layouts
            .get(key.as_ref())
            .and_then(|layout| layout.field_constness(field, &*layouts))
            .is_some_and(|(base_const, _, _)| base_const)
    }

    fn call_const_base(&self, callee: &Expr) -> Option<bool> {
        match callee {
            Expr::Identifier(name, _) => self.functions.get(name).map(|f| f.return_base_const),
            _ => None,
        }
    }
}

impl<'a> ExprTypeChecker<'a> {
    /// The value-context type, keeping object qualifiers when arrays decay.
    /// Raw inference deliberately retains arrays for sizeof, &, and member
    /// selection. All value consumers use this boundary rather than recreating
    /// an unqualified pointer independently. Existing expression memoization
    /// remains authoritative for each raw expression type.
    pub(super) fn infer_value_ctype(&self, expr: &Expr) -> Option<CType> {
        let raw = self.infer_expr_ctype(expr)?;
        let array = matches!(raw, CType::Array(..));
        let mut value = raw.decayed_value_ctype();
        if array {
            if let CType::Pointer(_, quals) = &mut value {
                *quals = quals
                    .in_space(self.lvalue_address_space(expr))
                    .with_cvr(self.lvalue_object_cvr(expr));
            }
        }
        Some(value)
    }

    /// The complete own CVR bits of the object the lvalue `expr` designates,
    /// when exactly known: a variable's record; a member's own qualifiers
    /// joined with its containing object's (C11 6.5.2.3p3, `.` and `->`
    /// alike); and the pointee record of the pointer that `*`, `->` or a
    /// subscript goes through (an array operand: the array object's own,
    /// which are its elements'). `None` when any link is unknown.
    fn lvalue_object_cvr(&self, expr: &Expr) -> Option<u8> {
        match expr {
            Expr::Identifier(name, _) => self.symbols.object_cvr(name),
            Expr::MemberAccess(base, field, _) => {
                let outer = self.lvalue_object_cvr(base)?;
                let record = self.infer_expr_ctype(base)?;
                Some(outer | self.member_own_cvr(&record, field)?)
            }
            Expr::PointerMemberAccess(base, field, _) => {
                let (record, outer) = self.designated_pointee(base)?;
                Some(outer | self.member_own_cvr(&record, field)?)
            }
            Expr::Deref(inner, _) => self.designated_pointee(inner).map(|(_, cvr)| cvr),
            Expr::ArraySubscript(lhs, rhs, _) => {
                let lhs_indexes = self
                    .infer_expr_ctype(lhs)
                    .is_some_and(|t| matches!(t, CType::Pointer(..) | CType::Array(..)));
                let base = if lhs_indexes { lhs } else { rhs };
                self.designated_pointee(base).map(|(_, cvr)| cvr)
            }
            Expr::GenericSelection(controlling, associations, _) => {
                let index = self.generic_selection_index(controlling, associations)?;
                self.lvalue_object_cvr(&associations[index].expr)
            }
            Expr::FunctionCall(callee, args, _)
                if matches!(callee.as_ref(), Expr::Identifier(name, _)
                    if name == "__builtin_choose_expr")
                    && args.len() >= 3 =>
            {
                let chosen = if self.choose_expr_takes_first(&args[0]) {
                    1
                } else {
                    2
                };
                self.lvalue_object_cvr(&args[chosen])
            }
            _ => None,
        }
    }

    /// The object type `expr` points at (an array operand: its element) and
    /// that object's complete own CVR bits, when exactly known.
    fn designated_pointee(&self, expr: &Expr) -> Option<(CType, u8)> {
        match self.infer_expr_ctype(expr)? {
            CType::Pointer(pointee, quals) => Some((*pointee, quals.cvr()?)),
            CType::Array(elem, _) => Some((*elem, self.lvalue_object_cvr(expr)?)),
            _ => None,
        }
    }

    /// Member `field`'s own CVR bits in record type `record` (see
    /// `StructLayout::member_own_cvr`).
    fn member_own_cvr(&self, record: &CType, field: &str) -> Option<u8> {
        let (CType::Struct(key) | CType::Union(key)) = record else {
            return None;
        };
        let layouts = self.types.borrow_struct_layouts();
        layouts.get(&key[..])?.member_own_cvr(field, &*layouts)
    }

    /// Infer the CType of an expression.
    ///
    /// Returns `Some(CType)` when the type can be determined from sema state,
    /// `None` when the type depends on lowering-specific information (e.g.,
    /// complex expression chains through typeof).
    pub fn infer_expr_ctype(&self, expr: &Expr) -> Option<CType> {
        // Memoization: if this expression's type was already computed during
        // the bottom-up sema walk, return it in O(1) instead of re-traversing.
        // This prevents O(2^N) blowup on deep expression chains.
        if let Some(cache) = self.expr_types {
            if let Some(cached) = cache.get(&expr.id()) {
                return Some(cached.clone());
            }
        }

        match expr {
            // Literals have well-defined types
            Expr::IntLiteral(val, _) => {
                // After the lexer fix, IntLiteral should only contain values
                // that fit in int. But defensively handle larger values too:
                // C11 6.4.4.1: decimal without suffix: int -> long -> long long
                if *val >= i32::MIN as i64 && *val <= i32::MAX as i64 {
                    Some(CType::Int)
                } else if crate::common::types::target_is_32bit() {
                    // ILP32: int and long are both 32-bit, promote to long long
                    Some(CType::LongLong)
                } else {
                    // LP64: long is 64-bit
                    Some(CType::Long)
                }
            }
            Expr::CharLiteral(_, _) => Some(CType::Int),
            Expr::UIntLiteral(val, _) => {
                if crate::common::types::target_is_32bit() && *val > u32::MAX as u64 {
                    Some(CType::ULongLong)
                } else {
                    Some(CType::UInt)
                }
            }
            Expr::LongLiteral(val, _) => {
                // On ILP32, long is 32-bit. If value doesn't fit, promote to long long.
                // C11 6.4.4.1: for 'l' suffix, type is: long, long long (decimal)
                //               or: long, unsigned long, long long, unsigned long long (hex/octal)
                if crate::common::types::target_is_32bit() {
                    if *val >= i32::MIN as i64 && *val <= i32::MAX as i64 {
                        Some(CType::Long) // fits in 32-bit long
                    } else {
                        Some(CType::LongLong) // promote to 64-bit long long
                    }
                } else {
                    Some(CType::Long)
                }
            }
            Expr::ULongLiteral(val, _) => {
                // On ILP32, unsigned long is 32-bit. If value doesn't fit, promote to unsigned long long.
                if crate::common::types::target_is_32bit() && *val > u32::MAX as u64 {
                    Some(CType::ULongLong)
                } else {
                    Some(CType::ULong)
                }
            }
            // long long is always 64-bit, regardless of target
            Expr::LongLongLiteral(_, _) => Some(CType::LongLong),
            Expr::ULongLongLiteral(_, _) => Some(CType::ULongLong),
            Expr::FloatLiteral(_, _) => Some(CType::Double),
            Expr::FloatLiteralF32(_, _) => Some(CType::Float),
            Expr::FloatLiteralLongDouble(_, _, _) => Some(CType::LongDouble),
            Expr::FloatLiteralF128(_, _, _) => Some(CType::Float128),
            Expr::FloatLiteralDecimal(width, _, _) => match width {
                32 => Some(CType::Decimal32),
                64 => Some(CType::Decimal64),
                _ => Some(CType::Decimal128),
            },
            Expr::FloatLiteralHalf(is_bf16, _, _) => Some(if *is_bf16 {
                CType::BFloat16
            } else {
                CType::Float16
            }),
            Expr::ImaginaryLiteral(_, _) => Some(CType::ComplexDouble),
            Expr::ImaginaryLiteralF32(_, _) => Some(CType::ComplexFloat),
            Expr::ImaginaryLiteralLongDouble(_, _, _) => Some(CType::ComplexLongDouble),
            Expr::StringLiteral(_, _) => Some(CType::Pointer(
                Box::new(CType::plain_char()),
                PtrQuals::of(AddressSpace::Default),
            )),
            Expr::WideStringLiteral(_, _) => {
                Some(CType::Pointer(Box::new(CType::Int), PtrQuals::GENERIC))
            }
            Expr::Char16StringLiteral(_, _) => Some(CType::Pointer(
                Box::new(CType::UShort),
                PtrQuals::of(AddressSpace::Default),
            )),

            // Identifiers: look up in symbol table or enum constants
            Expr::Identifier(name, _) => {
                // Inside a function the body scope declares it (an array of
                // the name's length plus one); at file scope it is GCC's
                // string for that spelling.
                if crate::frontend::parser::ast::is_function_name_builtin(name)
                    && self.symbols.lookup(name).is_none()
                {
                    let text = crate::frontend::parser::ast::function_name_builtin_text(name, None);
                    return Some(CType::Array(
                        Box::new(CType::plain_char()),
                        Some(text.len() + 1),
                    ));
                }
                // Objects/functions and enumerators share C's ordinary-name
                // namespace (C11 6.2.3): the innermost visible declaration
                // decides. Sema enters every enumerator in the symbol table as
                // well; one known only to `enum_constants` (a block-scope enum
                // seen by lowering's type context) applies when no symbol of
                // that name is visible at all.
                match self.symbols.lookup_ordinary(name) {
                    Some((sym, false)) => return Some(sym.ty.clone()),
                    found => {
                        if let Some(constant) = self.types.enum_constants.get(name) {
                            return Some(constant.ty.clone());
                        }
                        if let Some((sym, _)) = found {
                            return Some(sym.ty.clone());
                        }
                    }
                }
                // Check function signatures for implicitly declared functions
                if let Some(func_info) = self.functions.get(name) {
                    return Some(CType::Function(Box::new(func_info.call_type())));
                }
                None
            }

            // Cast: type is the target type
            Expr::Cast(type_spec, _, _, quals) => Some(self.cast_ctype(type_spec, quals)),

            // Sizeof, Countof, and Alignof always produce size_t (unsigned long on 64-bit)
            Expr::Sizeof(_, _)
            | Expr::Countof(_, _)
            | Expr::Alignof(_, _)
            | Expr::AlignofExpr(_, _)
            | Expr::GnuAlignof(_, _)
            | Expr::GnuAlignofExpr(_, _) => Some(CType::ULong),

            // Address-of wraps in Pointer
            Expr::AddressOf(inner, _) => {
                // `&e` points into the object `e` designates, so it carries that
                // object's space (`&gobj` of a `__seg_gs` object is `__seg_gs *`).
                let space = self.lvalue_object_space(inner);
                if let Some(inner_ct) = self.infer_expr_ctype(inner) {
                    // C11 6.5.3.2p3: `&E` is a pointer to the type of E, and a
                    // named address space is part of that storage, not a
                    // property of the value. Without it, `&percpu_var` typed as
                    // a generic pointer and a call to the `__percpu`-qualified
                    // function it belongs to was rejected (kernel/softirq.c).
                    let space = self.lvalue_address_space(inner);
                    Some(CType::Pointer(
                        Box::new(inner_ct),
                        PtrQuals::of(space).with_cvr(self.lvalue_object_cvr(inner)),
                    ))
                } else {
                    // Even if inner type unknown, result is some pointer
                    Some(CType::Pointer(Box::new(CType::Void), PtrQuals::of(space)))
                }
            }

            // Dereference peels off one Pointer/Array layer
            Expr::Deref(inner, _) => {
                if let Some(inner_ct) = self.infer_expr_ctype(inner) {
                    match inner_ct {
                        CType::Pointer(pointee, _) => Some(*pointee),
                        CType::Array(elem, _) => Some(*elem),
                        // Dereferencing a function is a no-op in C
                        CType::Function(_) => Some(inner_ct),
                        _ => None,
                    }
                } else {
                    None
                }
            }

            // Array subscript peels off one Array/Pointer layer
            Expr::ArraySubscript(base, index, _) => {
                // Try base first (arr[i])
                if let Some(base_ct) = self.infer_expr_ctype(base) {
                    match base_ct {
                        CType::Array(elem, _) => return Some(*elem),
                        CType::Pointer(pointee, _) => return Some(*pointee),
                        // A vector lane has the element type (GNU vector_size
                        // and Clang ext_vector_type alike).
                        ref v if v.is_vector() => {
                            return v.vector_info().map(|(elem, _)| elem.clone());
                        }
                        _ => {}
                    }
                }
                // Reverse subscript (i[arr])
                if let Some(idx_ct) = self.infer_expr_ctype(index) {
                    match idx_ct {
                        CType::Array(elem, _) => return Some(*elem),
                        CType::Pointer(pointee, _) => return Some(*pointee),
                        _ => {}
                    }
                }
                None
            }

            // Member access: look up field type in struct layout
            Expr::MemberAccess(base_expr, field_name, _) => {
                self.infer_field_ctype(base_expr, field_name, false)
            }
            Expr::PointerMemberAccess(base_expr, field_name, _) => {
                self.infer_field_ctype(base_expr, field_name, true)
            }

            // Unary operators
            Expr::UnaryOp(op, inner, _) => match op {
                UnaryOp::LogicalNot => Some(CType::Int),
                UnaryOp::Neg | UnaryOp::Plus | UnaryOp::BitNot => {
                    if let Some(inner_ct) = self.infer_expr_ctype(inner) {
                        if inner_ct.is_integer() {
                            Some(self.bitfield_promoted_ctype(inner, &inner_ct))
                        } else {
                            Some(inner_ct)
                        }
                    } else {
                        None
                    }
                }
                UnaryOp::PreInc | UnaryOp::PreDec => self.infer_expr_ctype(inner),
                UnaryOp::RealPart | UnaryOp::ImagPart => self
                    .infer_expr_ctype(inner)
                    .map(|inner_ct| inner_ct.complex_component_type()),
            },

            // Postfix operators preserve the operand type
            Expr::PostfixOp(_, inner, _) => self.infer_expr_ctype(inner),

            // Binary operators
            Expr::BinaryOp(op, lhs, rhs, _) => self.infer_binop_ctype(op, lhs, rhs),

            // Assignment: type of the left-hand side
            Expr::Assign(lhs, _, _) | Expr::CompoundAssign(_, lhs, _, _) => {
                self.infer_expr_ctype(lhs)
            }

            // Conditional: C11 6.5.15 composite type rules
            Expr::Conditional(_, then_expr, else_expr, _) => {
                self.conditional_ctype(then_expr, else_expr)
            }
            Expr::GnuConditional(cond, else_expr, _) => self.conditional_ctype(cond, else_expr),

            // Comma: the result is the right operand's value and type
            // (C11 6.5.17p3) AFTER the usual lvalue conversion, so an array
            // operand decays to a pointer exactly as in every other value
            // context (C11 6.3.2.1p3). Returning the raw right-hand type broke
            // that: for `struct s { char c[17]; }; sizeof(0, f().c)` reported
            // 17 (the ARRAY size) while codegen already materialised the
            // decayed address -- GCC reports 8, i.e. sizeof(char *). The two
            // halves of the compiler must agree on a type; sema saying "array"
            // while codegen says "pointer" is a latent bug in every pass that
            // trusts the inferred type. The conversion lives in
            // `CType::decayed_value_ctype` so sema, the sizeof/alignof sizing
            // paths and the lowerer's type maps share one implementation.
            Expr::Comma(_, rhs, _) => self.infer_value_ctype(rhs),

            // Function call: determine return type
            Expr::FunctionCall(func, args, _) => {
                // __builtin_choose_expr(const_expr, expr1, expr2) has the type
                // of the selected branch, not a fixed return type.
                if let Expr::Identifier(name, _) = func.as_ref() {
                    if name == "__builtin_choose_expr" && args.len() >= 3 {
                        return if self.choose_expr_takes_first(&args[0]) {
                            self.infer_expr_ctype(&args[1])
                        } else {
                            self.infer_expr_ctype(&args[2])
                        };
                    }

                    // `__builtin_shuffle` (GCC vector extension): the result
                    // is the FIRST vector operand's type — verified against
                    // gcc -O0 with _Generic (a `v4si` shuffle is a `v4si`, an
                    // `v8hi` shuffle an `v8hi`, in the 2- and 3-argument
                    // forms).  The name is not in `builtin_return_ctype`,
                    // which is keyed by name alone and cannot express an
                    // argument-dependent result, so before this arm the call
                    // had NO CType at all and every consumer that needs one
                    // treated a 16-byte vector as an untyped scalar:
                    // sizeof(__builtin_shuffle(v, m)) was 8, _Generic matched
                    // nothing, subscripting read the wrong lane, and — the
                    // miscompile — the lowering's by-value return path handed
                    // back the shuffle alloca's ADDRESS where the caller
                    // expected the 16 bytes of data.
                    //
                    // Both shuffle builtins use the ordinary pointer-backed
                    // vector value convention. `__builtin_shuffle` returns
                    // the first vector's type; `__builtin_shufflevector`
                    // returns that element type with one lane per index
                    // (GCC 16.2: `(v8si, v8si, 0, 2, 4, 6)` is a `v4si`).
                    if matches!(
                        name.as_str(),
                        "__builtin_shuffle" | "__builtin_shufflevector"
                    ) && !args.is_empty()
                    {
                        if let Some(result) = self.infer_expr_ctype(&args[0]).and_then(|first| {
                            CType::shuffle_builtin_result(name, &first, args.len())
                        }) {
                            return Some(result);
                        }
                    }

                    if let Some(ct) = self.infer_c2y_builtin_ctype(name, args) {
                        return Some(ct);
                    }
                }
                self.infer_call_return_ctype(func)
            }

            // VaArg, ConvertVector, BitCast, and CompoundLiteral: type from type specifier
            Expr::VaArg(_, type_spec, _)
            | Expr::ConvertVector(_, type_spec, _)
            | Expr::BitCast(type_spec, _, _) => Some(self.resolve_type_spec(type_spec)),
            Expr::CompoundLiteral(type_spec, init, _, _) => {
                let ct = self.resolve_type_spec(type_spec);
                match ct {
                    CType::Array(elem, None) => {
                        let count = match init.as_ref() {
                            crate::frontend::parser::ast::Initializer::List(items) => {
                                Some(items.len())
                            }
                            _ => None,
                        };
                        Some(CType::Array(elem, count))
                    }
                    other => Some(other),
                }
            }

            // Statement expression: the type of its value expression.
            //
            // A tail that cannot produce a value is `void` (GCC 16.2:
            // `({ int y; })`, `({ ; })`, `({ if (c) ; })`), never a
            // convenient default `int`: substituting one would let a use of a
            // void statement expression compile into an undefined value.
            Expr::StmtExpr(compound, _) => {
                let Some(expr) = compound.value_expr() else {
                    return Some(CType::Void);
                };
                // Nested statement expression: resolve it scope-threaded
                // FIRST.  Plain inference of a not-yet-executed nested
                // stmt-expr can "succeed" with a wrong fallback type
                // (typeof failures degrade to Int, so `typeof(*p)` on an
                // unresolved nested stmt-expr yields int *), which then
                // poisons the outer typeof.  The scope-threaded path
                // re-derives declaration types from the AST and resolves
                // cross-compound references (kernel xchg/cmpxchg macros:
                // outer declares __ai_ptr, inner uses typeof(*__ai_ptr)).
                if let Expr::StmtExpr(inner_compound, _) = expr {
                    let scope = self.build_compound_scope(compound, None);
                    if !scope.is_empty()
                        && let Some(ctype) =
                            self.infer_stmt_expr_ctype_with_scope(inner_compound, Some(&scope))
                    {
                        return Some(ctype);
                    }
                }
                if let Some(ctype) = self.infer_expr_ctype(expr) {
                    return Some(ctype);
                }
                // If the value expression is an identifier not in the symbol
                // table (e.g., inside typeof where the stmt expr was never
                // executed), resolve it from declarations within this compound
                // statement.  We build a local scope as we iterate so that
                // typeof expressions referencing earlier declarations can be
                // resolved (e.g., the kernel's xchg macro: typeof(&field) ptr
                // = ...; __typeof__(*ptr) ret = ...; ret;)
                if let Expr::Identifier(name, _) = expr {
                    return self.resolve_var_from_compound(compound, name, None);
                }
                None
            }

            // _Generic: resolve based on controlling expression type
            Expr::GenericSelection(controlling, associations, _) => {
                self.infer_generic_selection_ctype(controlling, associations)
            }

            // Label address: void*
            Expr::TypeOperand(ts, _, _) => Some(self.resolve_type_spec(ts)),
            Expr::LabelAddr(_, _) => Some(CType::Pointer(Box::new(CType::Void), PtrQuals::GENERIC)),

            // __builtin_types_compatible_p: int
            Expr::BuiltinTypesCompatibleP(_, _, _, _, _) => Some(CType::Int),
        }
    }

    /// Infer the CType of a binary operation.
    ///
    /// Evaluates each operand's type at most once to avoid exponential blowup
    /// on deeply nested expression chains like `+1+1+1+...+1` (which appear in
    /// preprocessor-generated enum initializers).
    fn infer_binop_ctype(&self, op: &BinOp, lhs: &Expr, rhs: &Expr) -> Option<CType> {
        match op {
            // A GNU vector comparison yields a vector of signed integers with
            // the operand's lane width and count (GCC: `int` lanes for
            // `float`, 64-bit lanes for `double`); a scalar comparison and
            // the logical operators produce `int`.
            BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                // Two stack slots, no heap allocation on every scalar compare.
                let operands = [self.infer_expr_ctype(lhs), self.infer_expr_ctype(rhs)];
                let vector = operands.iter().flatten().find(|t| t.is_vector());
                // A vector against a pointer is an invalid operand pair (GCC's
                // `error_mark`): leave it untyped so no conversion diagnostic
                // cascades from the already-reported error.
                if vector.is_some()
                    && operands.iter().flatten().any(|t| {
                        matches!(
                            t,
                            CType::Pointer(..) | CType::Array(..) | CType::Function(..)
                        )
                    })
                {
                    return None;
                }
                return match vector {
                    Some(v) => v.vector_comparison_result(),
                    None => Some(CType::Int),
                };
            }
            BinOp::LogicalAnd | BinOp::LogicalOr => return Some(CType::Int),
            _ => {}
        }

        // Evaluate each operand type once and reuse the results.
        let lct = self
            .infer_expr_ctype(lhs)
            .map(|ct| self.bitfield_promoted_if_bitfield(lhs, ct));
        let rct = self
            .infer_expr_ctype(rhs)
            .map(|ct| self.bitfield_promoted_if_bitfield(rhs, ct));

        // GNU vector shifts are element-wise when either operand is a vector.
        // For `scalar << vector` the scalar is splatted and the result has the
        // RHS vector type; only scalar shifts use the promoted-lhs rule.
        if matches!(op, BinOp::Shl | BinOp::Shr) {
            if rct.as_ref().is_some_and(CType::is_vector) {
                return rct;
            }
            if let Some(l) = lct {
                return Some(l.integer_promoted());
            }
            return Some(CType::Int);
        }

        // Pointer arithmetic for Add and Sub (including function designators
        // decaying to function pointers under GNU C pointer-arithmetic rules).
        let is_ptr_or_fn = |ct: &CType| ct.is_pointer_like() || matches!(ct, CType::Function(_));
        if matches!(op, BinOp::Add | BinOp::Sub) {
            if let Some(ref l) = lct {
                match l {
                    CType::Pointer(_, _) => {
                        if *op == BinOp::Sub && rct.as_ref().is_some_and(is_ptr_or_fn) {
                            return Some(CType::Long);
                        }
                        return lct;
                    }
                    CType::Array(elem, _) => {
                        if *op == BinOp::Sub && rct.as_ref().is_some_and(is_ptr_or_fn) {
                            return Some(CType::Long);
                        }
                        // The decayed array keeps the space of its object:
                        // `long __seg_gs a[8]; a + 4` points into %gs.
                        let space = self
                            .operand_memory_space(lhs)
                            .unwrap_or(AddressSpace::Default);
                        return Some(CType::Pointer(
                            elem.clone(),
                            PtrQuals::of(space).with_cvr(self.lvalue_object_cvr(lhs)),
                        ));
                    }
                    CType::Function(_) => {
                        if *op == BinOp::Sub && rct.as_ref().is_some_and(is_ptr_or_fn) {
                            return Some(CType::Long);
                        }
                        return Some(CType::Pointer(Box::new(l.clone()), PtrQuals::GENERIC));
                    }
                    _ => {}
                }
            }
            if *op == BinOp::Add {
                // int + ptr case
                if let Some(ref r) = rct {
                    match r {
                        CType::Pointer(_, _) => return rct,
                        CType::Array(elem, _) => {
                            let space = self
                                .operand_memory_space(rhs)
                                .unwrap_or(AddressSpace::Default);
                            return Some(CType::Pointer(
                                elem.clone(),
                                PtrQuals::of(space).with_cvr(self.lvalue_object_cvr(rhs)),
                            ));
                        }
                        CType::Function(_) => {
                            return Some(CType::Pointer(
                                Box::new(r.clone()),
                                PtrQuals::of(AddressSpace::Default),
                            ));
                        }
                        _ => {}
                    }
                }
            }
        }

        // GCC vector extensions: if either operand is a vector type, the result
        // is that vector type.  usual_arithmetic_conversion doesn't handle vectors.
        if let Some(ref l) = lct {
            if l.is_vector() {
                return lct;
            }
        }
        if let Some(ref r) = rct {
            if r.is_vector() {
                return rct;
            }
        }

        // Arithmetic/bitwise: usual arithmetic conversions
        match (lct, rct) {
            (Some(l), Some(r)) => Some(CType::usual_arithmetic_conversion(&l, &r)),
            (Some(l), None) => Some(l.integer_promoted()),
            (None, Some(r)) => Some(r.integer_promoted()),
            (None, None) => None,
        }
    }

    /// Infer the return CType of a function call.
    fn infer_call_return_ctype(&self, func: &Expr) -> Option<CType> {
        // Strip Deref layers (dereferencing function pointers is a no-op in C)
        let mut stripped = func;
        while let Expr::Deref(inner, _) = stripped {
            stripped = inner;
        }

        if let Expr::Identifier(name, _) = stripped {
            // The lexical binding first, the translation-unit record second.
            //
            // A block-scope declaration of a function -- a GNU nested function, or
            // a function-pointer variable -- shadows the file-scope entity of that
            // basename (C11 6.2.1p4), and this tree lowers the *call* to the
            // shadowing entity. Reading `functions[name]` first therefore typed the
            // call with the wrong entity's signature while emitting the call to the
            // right symbol: for `int f(int)` at file scope and a nested
            // `double f(double)` inside `g`, `g`'s `f(1.5)` was lowered as a
            // double-returning chain call and then converted `%xmm0`'s FP result
            // through `vcvtsi2sd` because the *expression type* said `int`. The ABI
            // and the value disagreed silently.
            //
            // The ordering is safe for the ordinary case: a file-scope function
            // declared `int f(int)` is in the scope table with exactly that type, so
            // preferring the scope cannot change its answer; where the scope holds
            // something else, that something else is what the name means.
            if let Some(sym) = self.symbols.lookup(name) {
                // A function-pointer object is also an ordinary-name binding:
                // it must shadow a same-named file-scope function even when
                // the latter has an incompatible return type. If the binding
                // is not callable, do not fall back to the hidden function.
                return Self::extract_return_ctype_from_type(&sym.ty);
            }
            // Check function signatures (external entities and implicit decls)
            if let Some(func_info) = self.functions.get(name.as_str()) {
                return Some(func_info.return_type.clone());
            }
            // Check builtin return types
            if let Some(ct) = Self::builtin_return_ctype(name) {
                return Some(ct);
            }
        }

        // For complex expressions (indirect calls through computed function pointers),
        // try to infer the function pointer's type
        if let Some(func_ct) = self.infer_expr_ctype(stripped) {
            return Self::extract_return_ctype_from_type(&func_ct);
        }

        None
    }

    /// Extract the return CType from a function or function pointer type.
    fn extract_return_ctype_from_type(ct: &CType) -> Option<CType> {
        match ct {
            CType::Function(ft) => Some(ft.return_type.clone()),
            CType::Pointer(inner, _) => match inner.as_ref() {
                CType::Function(ft) => Some(ft.return_type.clone()),
                // Pointer-to-function-pointer
                CType::Pointer(inner2, _) => match inner2.as_ref() {
                    CType::Function(ft) => Some(ft.return_type.clone()),
                    _ => Some(inner.as_ref().clone()),
                },
                other => Some(other.clone()),
            },
            _ => None,
        }
    }

    /// Infer the CType of a struct/union field access.
    fn infer_field_ctype(
        &self,
        base_expr: &Expr,
        field_name: &str,
        is_pointer: bool,
    ) -> Option<CType> {
        let base_ctype = if is_pointer {
            match self.infer_expr_ctype(base_expr)? {
                CType::Pointer(inner, _) => *inner,
                CType::Array(inner, _) => *inner,
                _ => return None,
            }
        } else {
            self.infer_expr_ctype(base_expr)?
        };

        match &base_ctype {
            CType::Struct(key) | CType::Union(key) => {
                let layouts = self.types.borrow_struct_layouts();
                if let Some(layout) = layouts.get(key.as_ref()) {
                    if let Some((_offset, field_ct)) = layout.field_offset(field_name, &*layouts) {
                        return Some(field_ct);
                    }
                }
                None
            }
            // Clang ext-vector component access: `v.x` is the element,
            // `v.xy` a two-lane vector of it.
            CType::ExtVector(..) => base_ctype.ext_vector_swizzle_ctype(field_name),
            _ => None,
        }
    }

    /// Look up the declared bit-field width when `expr` directly designates a
    /// struct/union bit-field member (including through anonymous records).
    pub fn expr_bitfield_width(&self, expr: &Expr) -> Option<u32> {
        let (base_expr, field_name, is_pointer) = match expr {
            Expr::MemberAccess(b, f, _) => (b.as_ref(), f.as_str(), false),
            Expr::PointerMemberAccess(b, f, _) => (b.as_ref(), f.as_str(), true),
            _ => return None,
        };
        let base_ctype = if is_pointer {
            match self.infer_expr_ctype(base_expr)? {
                CType::Pointer(inner, _) | CType::Array(inner, _) => *inner,
                _ => return None,
            }
        } else {
            self.infer_expr_ctype(base_expr)?
        };
        let (CType::Struct(key) | CType::Union(key)) = base_ctype else {
            return None;
        };
        let layouts = self.types.borrow_struct_layouts();
        layouts
            .get(key.as_ref())?
            .field_offset_with_bitfield(field_name, &*layouts)?
            .3
    }

    /// Apply C11 6.3.1.1p2 bit-field integer promotion ONLY if `expr` is a
    /// bit-field member access; otherwise leave `ct` unchanged.
    pub fn bitfield_promoted_if_bitfield(&self, expr: &Expr, ct: CType) -> CType {
        let Some(width) = self.expr_bitfield_width(expr) else {
            return ct;
        };
        if width <= 31 {
            CType::Int
        } else if width == 32 {
            if ct.is_unsigned() {
                CType::UInt
            } else {
                CType::Int
            }
        } else {
            ct
        }
    }

    /// Integer promotion (`integer_promoted`) aware of bit-field widths.
    pub fn bitfield_promoted_ctype(&self, expr: &Expr, ct: &CType) -> CType {
        let Some(width) = self.expr_bitfield_width(expr) else {
            return ct.integer_promoted();
        };
        if width <= 31 {
            CType::Int
        } else if width == 32 {
            if ct.is_unsigned() {
                CType::UInt
            } else {
                CType::Int
            }
        } else {
            ct.integer_promoted()
        }
    }

    /// Resolve a _Generic selection based on the controlling expression's type.
    fn infer_generic_selection_ctype(
        &self,
        controlling: &Expr,
        associations: &[GenericAssociation],
    ) -> Option<CType> {
        let index = self.generic_selection_index(controlling, associations)?;
        self.infer_expr_ctype(&associations[index].expr)
    }

    /// The index of the association a `_Generic` selects (C11 6.5.1.1p2): the
    /// first whose type is compatible with the controlling expression's type,
    /// else the `default` association, else `None`. The controlling expression
    /// is not evaluated. This is the single selection rule for the type
    /// checker and the static-initializer check, so the two cannot disagree.
    pub fn generic_selection_index(
        &self,
        controlling: &Expr,
        associations: &[GenericAssociation],
    ) -> Option<usize> {
        // An expression operand undergoes lvalue conversion; a type operand
        // (C2y) none. Constness follows the rule lowering shares.
        let controlling_ct = match controlling {
            Expr::TypeOperand(ts, ..) => Some(self.resolve_type_spec(ts)),
            _ => self
                .infer_expr_ctype(controlling)
                .map(|ct| ct.decayed_value_ctype()),
        };
        let (ctrl_const, compare_const) =
            qualifiers::controlling_const(controlling, associations, self);
        let mut default_index = None;
        for (index, assoc) in associations.iter().enumerate() {
            match &assoc.type_spec {
                None => default_index = Some(index),
                Some(type_spec) => {
                    let assoc_ct = self.resolve_type_spec(type_spec);
                    if controlling_ct
                        .as_ref()
                        .is_some_and(|ct| ct.generic_association_matches(&assoc_ct))
                        && (!compare_const
                            || qualifiers::association_const_base(assoc, self) == ctrl_const)
                    {
                        return Some(index);
                    }
                }
            }
        }
        default_index
    }

    /// Resolve a TypeSpecifier to a CType using sema's type context.
    /// Shared with `SemaConstEval` so compile-time constant evaluation and
    /// expression type checking always produce identical `CType`s.
    /// The type a cast converts to, with the qualifiers of a plain star
    /// chain's pointer levels recorded (see `PtrQuals`).
    fn cast_ctype(&self, spec: &TypeSpecifier, quals: &CastQualifiers) -> CType {
        let mut ty = self.resolve_type_spec(spec);
        if let Some(levels) = quals.star_chain() {
            let mut leaf = spec;
            while let TypeSpecifier::Pointer(inner, _) = leaf {
                leaf = inner;
            }
            let base =
                crate::common::type_builder::specifier_cvr_with(leaf, quals.base_cvr, &|n| {
                    self.types.typedef_top_cvr(n)
                });
            crate::common::type_builder::record_pointer_quals(&mut ty, base, &levels);
        }
        ty
    }

    pub(super) fn resolve_type_spec(&self, spec: &TypeSpecifier) -> CType {
        // Use a simple inline resolution for common cases to avoid the trait dispatch
        match spec {
            TypeSpecifier::Void => CType::Void,
            TypeSpecifier::Bool => CType::Bool,
            TypeSpecifier::Char => CType::Char,
            TypeSpecifier::SignedChar => CType::SChar,
            TypeSpecifier::UnsignedChar => CType::UChar,
            TypeSpecifier::PlainUnsignedChar => CType::PlainUChar,
            TypeSpecifier::Short => CType::Short,
            TypeSpecifier::UnsignedShort => CType::UShort,
            TypeSpecifier::Int | TypeSpecifier::Signed => CType::Int,
            TypeSpecifier::UnsignedInt | TypeSpecifier::Unsigned => CType::UInt,
            TypeSpecifier::Long => CType::Long,
            TypeSpecifier::UnsignedLong => CType::ULong,
            TypeSpecifier::LongLong => CType::LongLong,
            TypeSpecifier::UnsignedLongLong => CType::ULongLong,
            TypeSpecifier::Int128 => CType::Int128,
            TypeSpecifier::UnsignedInt128 => CType::UInt128,
            TypeSpecifier::Float => CType::Float,
            TypeSpecifier::Double => CType::Double,
            TypeSpecifier::LongDouble => CType::LongDouble,
            TypeSpecifier::Float16 => CType::Float16,
            TypeSpecifier::BFloat16 => CType::BFloat16,
            TypeSpecifier::Float128 => CType::Float128,
            TypeSpecifier::Decimal32 => CType::Decimal32,
            TypeSpecifier::Decimal64 => CType::Decimal64,
            TypeSpecifier::Decimal128 => CType::Decimal128,
            TypeSpecifier::ComplexFloat => CType::ComplexFloat,
            TypeSpecifier::ComplexDouble => CType::ComplexDouble,
            TypeSpecifier::ComplexLongDouble => CType::ComplexLongDouble,
            TypeSpecifier::Pointer(inner, addr_space) => {
                // A pointer chain is a linear spine, so walking it is a loop,
                // not a recursion: see the same transform in
                // type_builder::resolve_type_spec_to_ctype. Three
                // near-duplicate TypeSpecifier->CType resolvers exist (this
                // one, const_eval::ctype_from_type_spec and type_builder's);
                // each recursed once per `*` and each could be driven to stack
                // overflow by a deeply-starred declarator. The recursive call
                // below handles only the innermost NON-pointer node, so
                // pointer depth costs no stack frames at any level.
                //
                // An unqualified innermost pointer over `typeof(expr)` takes
                // the space of `expr`, as in type_builder (GCC behaviour).
                let mut spaces = Vec::with_capacity(8);
                spaces.push(*addr_space);
                let mut cur: &TypeSpecifier = inner;
                while let TypeSpecifier::Pointer(i, s) = cur {
                    spaces.push(*s);
                    cur = i;
                }
                let leaf_space = self.typeof_pointee_space(cur);
                let mut ty = self.resolve_type_spec(cur);
                let innermost = spaces.len() - 1;
                for (i, space) in spaces.iter().enumerate().rev() {
                    let space = if i == innermost && *space == AddressSpace::Default {
                        leaf_space
                    } else {
                        *space
                    };
                    ty = CType::Pointer(Box::new(ty), PtrQuals::of(space));
                }
                ty
            }
            TypeSpecifier::Array(elem, size) => {
                let elem_ct = self.resolve_type_spec(elem);
                let arr_size = size.as_ref().and_then(|s| self.eval_const_expr(s));
                CType::Array(Box::new(elem_ct), arr_size.map(|s| s as usize))
            }
            TypeSpecifier::TypedefName(name) => {
                if let Some(resolved) = self.types.typedefs.get(name) {
                    resolved.clone()
                } else {
                    CType::Int // fallback for unknown typedef
                }
            }
            TypeSpecifier::Enum(name, variants, is_packed, underlying) => {
                // The same enum the analyzer resolves (`resolve_enum`): the
                // completed definition visible here, or this body's values.
                // Every integer property of the result (size, signedness,
                // rank, promotion, `_Generic` match) is that of its
                // compatible type, so no width rule lives here.
                CType::Enum(crate::common::type_builder::enum_type_of(
                    name,
                    variants,
                    *is_packed,
                    name.as_ref().and_then(|n| self.types.enum_types.get(n)),
                    underlying.as_deref().map(|u| self.resolve_type_spec(u)),
                    |v| v.value.as_ref().and_then(|e| self.eval_const_expr(e)),
                ))
            }
            TypeSpecifier::Struct(
                tag,
                fields,
                is_packed,
                pragma_pack,
                struct_aligned,
                reverse_sso,
            ) => {
                if let Some(tag) = tag {
                    CType::Struct(
                        self.types
                            .resolve_record_key(&format!("struct.{}", tag))
                            .into(),
                    )
                } else if let Some(fs) = fields {
                    // Without a tag name but with fields, register the anonymous
                    // struct layout so member access resolution works correctly
                    // (e.g., kernel get_unaligned() macro pattern with packed structs
                    // inside statement expressions).
                    self.resolve_anon_struct_or_union(
                        fs,
                        false,
                        *is_packed,
                        *pragma_pack,
                        *struct_aligned,
                        reverse_sso.is_some_and(|v| v),
                    )
                } else {
                    // Anonymous forward declaration (no tag, no fields)
                    CType::Int
                }
            }
            TypeSpecifier::Union(
                tag,
                fields,
                is_packed,
                pragma_pack,
                struct_aligned,
                reverse_sso,
            ) => {
                if let Some(tag) = tag {
                    CType::Union(
                        self.types
                            .resolve_record_key(&format!("union.{}", tag))
                            .into(),
                    )
                } else if let Some(fs) = fields {
                    // Same as struct: register anonymous union layout for member access
                    self.resolve_anon_struct_or_union(
                        fs,
                        true,
                        *is_packed,
                        *pragma_pack,
                        *struct_aligned,
                        reverse_sso.is_some_and(|v| v),
                    )
                } else {
                    // Anonymous forward declaration (no tag, no fields)
                    CType::Int
                }
            }
            TypeSpecifier::TypeofType(inner, _) => self.resolve_type_spec(inner),
            // A cast or type name spelled with `__attribute__((vector_size))`
            // (the parser wraps it, see `apply_pending_vector_attr`).
            TypeSpecifier::Vector(elem, total_bytes) => {
                CType::Vector(Box::new(self.resolve_type_spec(elem)), *total_bytes)
            }
            TypeSpecifier::Typeof(expr, _) => {
                // typeof(expr): try to infer the expression's type
                self.infer_expr_ctype(expr).unwrap_or(CType::Int)
            }
            TypeSpecifier::FunctionPointer(ret, params, variadic, has_prototype) => {
                let ret_ct = self.resolve_type_spec(ret);
                // Full per-parameter conversion (param_decl_to_ctype_with): a
                // nested function-pointer parameter inside the type name
                // (`void(*)(void *, int (*fun)(unsigned int))`) rides on
                // ParamDecl.fptr_params; resolving `p.type_spec` alone yields
                // `int *` and rejects GCC-legal calls (tracepoint
                // __traceiter_cpuhp_enter probe dispatch).
                let param_cts: Vec<(CType, Option<String>)> = params
                    .iter()
                    .map(|p| {
                        (
                            crate::common::type_builder::param_decl_to_ctype_qualified(
                                &|ts| self.resolve_type_spec(ts),
                                &|ts, written| {
                                    crate::common::type_builder::specifier_cvr_with(
                                        ts,
                                        written,
                                        &|name| self.types.typedef_top_cvr(name),
                                    )
                                },
                                p,
                            ),
                            p.name.clone(),
                        )
                    })
                    .collect();
                CType::Pointer(
                    Box::new(CType::Function(Box::new(FunctionType {
                        return_type: ret_ct,
                        params: param_cts,
                        variadic: *variadic,
                        has_prototype: *has_prototype,
                    }))),
                    PtrQuals::of(AddressSpace::Default),
                )
            }
            TypeSpecifier::BareFunction(ret, params, variadic, has_prototype) => {
                let ret_ct = self.resolve_type_spec(ret);
                let param_cts: Vec<(CType, Option<String>)> = params
                    .iter()
                    .map(|p| {
                        (
                            crate::common::type_builder::param_decl_to_ctype_qualified(
                                &|ts| self.resolve_type_spec(ts),
                                &|ts, written| {
                                    crate::common::type_builder::specifier_cvr_with(
                                        ts,
                                        written,
                                        &|name| self.types.typedef_top_cvr(name),
                                    )
                                },
                                p,
                            ),
                            p.name.clone(),
                        )
                    })
                    .collect();
                CType::Function(Box::new(FunctionType {
                    return_type: ret_ct,
                    params: param_cts,
                    variadic: *variadic,
                    has_prototype: *has_prototype,
                }))
            }
            TypeSpecifier::ExtVector(elem, lanes) => {
                let elem_ct = self.resolve_type_spec(elem);
                match crate::common::types::ExtVectorLayout::new(&elem_ct, *lanes) {
                    Some(layout) => CType::ExtVector(Box::new(elem_ct), layout),
                    // Same recovery as `type_builder`: the element type.
                    None => elem_ct,
                }
            }
            TypeSpecifier::NullPtrT => CType::NullPtr,
            // `__auto_type`/C23 `auto` is replaced by the initializer's type
            // before any expression can name it; `type_builder` agrees.
            TypeSpecifier::AutoType => CType::Int,
        }
    }

    /// The type of `c ? then_expr : else_expr` (C11 6.5.15p3-p6).
    fn conditional_ctype(&self, then_expr: &Expr, else_expr: &Expr) -> Option<CType> {
        let then_ct = self.infer_value_ctype(then_expr);
        let else_ct = self.infer_value_ctype(else_expr);
        let then_is_npc = self.conditional_arm_is_npc(then_expr, &then_ct, &else_ct);
        let else_is_npc = self.conditional_arm_is_npc(else_expr, &else_ct, &then_ct);
        CType::conditional_composite_type(then_ct, else_ct, then_is_npc, else_is_npc)
    }

    /// Whether a `?:` arm is a null pointer constant, decided only where the
    /// answer can change the type ([`CType::conditional_arm_npc_matters`]);
    /// every other pair skips the structural ICE walk.
    fn conditional_arm_is_npc(
        &self,
        arm: &Expr,
        arm_ct: &Option<CType>,
        other_ct: &Option<CType>,
    ) -> bool {
        CType::conditional_arm_npc_matters(arm_ct.as_ref(), other_ct.as_ref())
            && self.is_null_pointer_constant(arm)
    }

    /// Whether `expr` is an integer constant expression (C11 6.6p6): the
    /// structural test of `ice.rs` over this checker's types, enumerators
    /// and required-constant evaluation. The one ICE predicate behind
    /// `__builtin_choose_expr` and the null pointer constant rule.
    /// Whether `expr` is a named constant (C23 6.6p7) and of which kind: an
    /// enumeration constant, a `constexpr` object, or a member of one reached
    /// by `.` (subscripts and `->` are not named constants: GCC 16.2 rejects
    /// `a[1]` of a `constexpr` array in an ICE).
    pub(super) fn named_constant(&self, expr: &Expr) -> Option<super::ice::NamedConstant> {
        use super::ice::NamedConstant;
        let mut root = expr;
        while let Expr::MemberAccess(base, _, _) = root {
            root = base;
        }
        match root {
            Expr::CompoundLiteral(_, _, _, st) if st.is_constexpr => {}
            Expr::Identifier(name, _) => {
                if std::ptr::eq(root, expr) && self.symbols.is_enumerator(name) {
                    return Some(NamedConstant::Integer);
                }
                if !self.symbols.is_constexpr(name) {
                    return None;
                }
            }
            _ => return None,
        }
        let ty = self.infer_expr_ctype(expr)?;
        if ty.is_integer() {
            Some(NamedConstant::Integer)
        } else if ty.is_floating() {
            Some(NamedConstant::Floating)
        } else {
            None
        }
    }

    pub fn is_integer_constant_expr(&self, expr: &Expr) -> bool {
        let evaluator = self.sema_const_eval();
        let eval = |e: &Expr| evaluator.eval_required_constant(e);
        let ctype = |e: &Expr| self.infer_expr_ctype(e);
        let select = |controlling: &Expr, associations: &[GenericAssociation]| {
            self.generic_selection_index(controlling, associations)
        };
        let named_constant = |e: &Expr| self.named_constant(e);
        let is_builtin = |name: &str| super::builtins::is_builtin(name);
        super::ice::IceChecker::new(
            &eval,
            &ctype,
            &select,
            &named_constant,
            &is_builtin,
            self.pre_c99,
        )
        .is_ice(expr)
    }

    /// An integer constant expression with the value 0.
    fn is_integer_constant_zero(&self, expr: &Expr) -> bool {
        self.is_integer_constant_expr(expr)
            && self
                .sema_const_eval()
                .eval_required_constant(expr)
                .is_some_and(|c| c.is_zero())
    }

    /// Whether `expr` is a null pointer constant (C23 6.3.2.3p3): an integer
    /// constant expression with the value 0, one cast to `void *`, or the
    /// predefined constant `nullptr` (which the parser spells `(nullptr_t)0`
    /// through `TypeSpecifier::NullPtrT`, a specifier no source text names;
    /// a written `(nullptr_t)0` goes through the typedef and is not one).
    ///
    /// Exactness matters beyond diagnostics: the type of `?:` depends on it,
    /// and the kernel's `__is_constexpr(x)` -- `sizeof(int) == sizeof(*(8 ?
    /// ((void *)((long)(x) * 0l)) : (int *)8))` -- selects code through that
    /// type. The syntactic `const_arith` approximation knows no enumerators
    /// (GCC 16.2: `__is_constexpr(N)` is 1 for an enumerator `N`; lccc said
    /// 0 and took the other `__builtin_choose_expr` arm) and accepts any
    /// cast of 0, `(char *)0` included.
    pub fn is_null_pointer_constant(&self, expr: &Expr) -> bool {
        matches!(expr, Expr::Cast(TypeSpecifier::NullPtrT, ..))
            || self.is_integer_constant_zero(expr)
            || self.is_void_pointer_null_constant(expr)
    }

    /// The `void *` form of a null pointer constant: an integer constant
    /// zero cast to an unqualified `void *`, typedef names resolved (`(CNPT)0`
    /// with `typedef const void *CNPT;` is not one). GCC accepts exactly this
    /// shape where a named-address-space pointer is expected.
    pub fn is_void_pointer_null_constant(&self, expr: &Expr) -> bool {
        crate::common::const_arith::is_void_pointer_null_constant(
            expr,
            &|ts| {
                if let TypeSpecifier::TypedefName(name) = ts {
                    if self.types.qualified_pointee_typedefs.contains(name) {
                        return false;
                    }
                }
                self.resolve_type_spec(ts)
                    == CType::Pointer(Box::new(CType::Void), PtrQuals::GENERIC)
            },
            &|operand| self.is_integer_constant_zero(operand),
        )
    }

    /// Whether `__builtin_choose_expr` with condition `cond` selects its
    /// first operand ([`super::const_eval::SemaConstEval::eval_required_constant`];
    /// a condition that is no constant has been diagnosed by sema).
    fn choose_expr_takes_first(&self, cond: &Expr) -> bool {
        self.sema_const_eval()
            .eval_required_constant(cond)
            .is_none_or(|c| c.is_nonzero())
    }

    fn sema_const_eval(&self) -> super::const_eval::SemaConstEval<'_> {
        super::const_eval::SemaConstEval {
            types: self.types,
            symbols: self.symbols,
            functions: self.functions,
            const_values: None,
            expr_types: self.expr_types,
            builtin_policy: self.builtin_policy.clone(),
            folding_initializer: false,
            pre_c99: self.pre_c99,
            named_constants_only: false,
        }
    }

    /// Evaluate a constant expression for array sizes and similar contexts.
    /// Delegates to SemaConstEval for the full implementation, unifying
    /// const_eval across all sema modules.
    fn eval_const_expr(&self, expr: &Expr) -> Option<i64> {
        self.sema_const_eval().eval_const_expr(expr)?.to_i64()
    }

    /// Resolve an anonymous struct or union with inline field declarations.
    /// Generates a unique key, computes the layout, registers it in TypeContext,
    /// and returns the CType (Struct or Union).
    fn resolve_anon_struct_or_union(
        &self,
        fields: &[StructFieldDecl],
        is_union: bool,
        is_packed: bool,
        pragma_pack: Option<usize>,
        struct_aligned: Option<usize>,
        reverse_sso: bool,
    ) -> CType {
        use crate::common::types::{StructField, StructLayout};

        let struct_fields: Vec<StructField> = fields
            .iter()
            .filter_map(|f| {
                // An enum bit-field keeps the enum type; its signedness is the
                // enum's compatible integer type.
                let ty = self.resolve_field_ctype(f);
                // C11 6.7.2.1p13: unnamed non-aggregate members are constraint
                // violations GCC ignores entirely; drop from layout + init.
                if f.name.is_none()
                    && f.bit_width.is_none()
                    && !matches!(ty, CType::Struct(_) | CType::Union(_))
                {
                    return None;
                }
                let typedef_alignment = |ts: &TypeSpecifier| match ts {
                    TypeSpecifier::TypedefName(name) => {
                        self.types.typedef_alignments.get(name).copied()
                    }
                    _ => None,
                };
                let alignas_type = f.alignas_type.as_ref().map(|ts| {
                    let natural = self
                        .resolve_type_spec(ts)
                        .align_ctx(&*self.types.borrow_struct_layouts());
                    natural.max(typedef_alignment(ts).unwrap_or(0))
                });
                let alignment =
                    crate::common::types::strictest_alignment([f.alignment, alignas_type]);
                // A typedef-level `aligned` REPLACES the type's alignment
                // (it may lower it); see `StructField::type_align`.
                let type_align = typedef_alignment(&f.type_spec);
                Some(StructField {
                    own_cvr: None,
                    name: f.name.clone().unwrap_or_default(),
                    ty,
                    bit_width: f
                        .bit_width
                        .as_ref()
                        .and_then(|bw| self.eval_const_expr(bw).map(|v| v as u32)),
                    alignment,
                    type_align,
                    is_packed: f.is_packed,
                    base_volatile: f.base_volatile,
                    base_const: qualifiers::type_name_const_base(&f.type_spec, f.base_const, self),
                    object_volatile: f.object_volatile,
                    object_const: f.object_const,
                    counted_by: f.counted_by.clone(),
                    pointer_quals: Vec::new(),
                })
            })
            .collect();

        let max_field_align = if is_packed { Some(1) } else { pragma_pack };
        let mut layout = if is_union {
            StructLayout::for_union_with_packing(
                &struct_fields,
                max_field_align,
                &*self.types.borrow_struct_layouts(),
                reverse_sso,
            )
        } else {
            StructLayout::for_struct_with_packing(
                &struct_fields,
                max_field_align,
                &*self.types.borrow_struct_layouts(),
                reverse_sso,
            )
        };

        // Apply struct-level __attribute__((aligned(N)))
        layout.apply_struct_aligned(struct_aligned);

        let id = self.types.next_anon_struct_id();
        let key = format!("__anon_struct_{}", id);
        self.types
            .insert_struct_layout_scoped_from_ref(&key, layout);
        if is_union {
            CType::Union(key.into())
        } else {
            CType::Struct(key.into())
        }
    }

    /// Get the CType for a struct field declaration, accounting for derived declarators.
    fn resolve_field_ctype(&self, f: &StructFieldDecl) -> CType {
        let base = self.resolve_type_spec(&f.type_spec);
        crate::common::type_builder::build_full_ctype_with_callbacks(
            base,
            &f.derived,
            &|ts| self.resolve_type_spec(ts),
            &|e, _| self.eval_const_expr(e).map(|v| v as usize),
        )
    }

    /// Return CType for known builtins.
    /// This is the CType-level equivalent of the lowerer's builtin_return_type (IrType).
    /// TODO: consolidate with sema/builtins.rs BuiltinInfo to avoid duplication.
    pub(crate) fn builtin_return_ctype(name: &str) -> Option<CType> {
        if let Some(result) = crate::common::x86_builtin_types::result_type(name) {
            return Some(result);
        }
        if let Some(op) = crate::common::avx_widen::AvxWiden::raw_builtin(name) {
            return Some(CType::Vector(Box::new(op.result_element()), 32));
        }
        match name {
            // BMI2 BZHI returns its operand type.
            "__builtin_ia32_bzhi_si" | "__builtin_ia32_pdep_si" | "__builtin_ia32_pext_si" => {
                Some(CType::UInt)
            }
            "__builtin_ia32_bzhi_di" | "__builtin_ia32_pdep_di" | "__builtin_ia32_pext_di" => {
                Some(CType::ULongLong)
            }
            // Float-returning builtins
            "__builtin_inf"
            | "__builtin_huge_val"
            | "__builtin_nan"
            | "__builtin_nans"
            | "__builtin_fma"
            | "__builtin_trunc"
            | "__builtin_rint"
            | "__builtin_nearbyint"
            | "__builtin_roundeven"
            | "__builtin_fabs"
            | "__builtin_sqrt"
            | "__builtin_sin"
            | "__builtin_cos"
            | "__builtin_log"
            | "__builtin_log2"
            | "__builtin_exp"
            | "__builtin_pow"
            | "__builtin_floor"
            | "__builtin_ceil"
            | "__builtin_round"
            | "__builtin_fmin"
            | "__builtin_fmax"
            | "__builtin_copysign"
            | "__builtin_nextafter" => Some(CType::Double),

            "__builtin_inff"
            | "__builtin_huge_valf"
            | "__builtin_nanf"
            | "__builtin_nansf"
            | "__builtin_fmaf"
            | "__builtin_truncf"
            | "__builtin_rintf"
            | "__builtin_nearbyintf"
            | "__builtin_roundevenf"
            | "__builtin_fabsf"
            | "__builtin_sqrtf"
            | "__builtin_sinf"
            | "__builtin_cosf"
            | "__builtin_logf"
            | "__builtin_expf"
            | "__builtin_powf"
            | "__builtin_floorf"
            | "__builtin_ceilf"
            | "__builtin_roundf"
            | "__builtin_copysignf"
            | "__builtin_nextafterf" => Some(CType::Float),

            "__builtin_infl"
            | "__builtin_huge_vall"
            | "__builtin_nanl"
            | "__builtin_nansl"
            | "__builtin_fabsl"
            | "__builtin_nextafterl" => Some(CType::LongDouble),

            // _Float16 & __bf16 builtins (GCC 16.2 / C23 TS 18661-3).
            "__builtin_inff16"
            | "__builtin_huge_valf16"
            | "__builtin_nanf16"
            | "__builtin_nansf16"
            | "__builtin_fabsf16"
            | "__builtin_copysignf16"
            | "__builtin_ceilf16"
            | "__builtin_floorf16"
            | "__builtin_truncf16"
            | "__builtin_roundf16"
            | "__builtin_roundevenf16"
            | "__builtin_rintf16"
            | "__builtin_nearbyintf16"
            | "__builtin_sqrtf16"
            | "__builtin_fmaf16"
            | "__builtin_fminf16"
            | "__builtin_fmaxf16"
            | "__builtin_nextafterf16" => Some(CType::Float16),
            "__builtin_nansf16b" => Some(CType::BFloat16),

            // _Float128 builtins (sema/builtins.rs): binary128 results.
            "__builtin_inff128"
            | "__builtin_huge_valf128"
            | "__builtin_nanf128"
            | "__builtin_nansf128"
            | "__builtin_copysignf128"
            | "__builtin_fabsf128"
            | "__builtin_infq"
            | "__builtin_huge_valq"
            | "__builtin_nanq"
            | "__builtin_nansq"
            | "__builtin_copysignq"
            | "__builtin_fabsq" => Some(CType::Float128),

            // Integer-returning builtins
            "__builtin_fpclassify"
            | "__builtin_isnan"
            | "__builtin_isnanf"
            | "__builtin_isnanl"
            | "__builtin_isinf"
            | "__builtin_isinff"
            | "__builtin_isinfl"
            | "__builtin_isfinite"
            | "__builtin_isfinitef"
            | "__builtin_isfinitel"
            | "__builtin_isnormal"
            | "__builtin_isnormalf"
            | "__builtin_isnormall"
            | "__builtin_signbit"
            | "__builtin_signbitf"
            | "__builtin_signbitl"
            | "__builtin_isinf_sign"
            | "__builtin_isgreater"
            | "__builtin_isgreaterequal"
            | "__builtin_isless"
            | "__builtin_islessequal"
            | "__builtin_islessgreater"
            | "__builtin_isunordered"
            | "__builtin_clz"
            | "__builtin_clzl"
            | "__builtin_clzll"
            | "__builtin_ctz"
            | "__builtin_ctzl"
            | "__builtin_ctzll"
            | "__builtin_clrsb"
            | "__builtin_clrsbl"
            | "__builtin_clrsbll"
            | "__builtin_popcount"
            | "__builtin_popcountl"
            | "__builtin_popcountll"
            | "__builtin_parity"
            | "__builtin_parityl"
            | "__builtin_parityll"
            | "__builtin_ffs"
            | "__builtin_ffsl"
            | "__builtin_ffsll"
            | "__builtin_types_compatible_p"
            | "__builtin_classify_type"
            | "__builtin_constant_p"
            | "__builtin_object_size" => Some(CType::Int),

            // Byte-swap builtins return unsigned types (GCC behavior)
            "__builtin_bswap16" | "__builtin_bswap32" => Some(CType::UInt),
            "__builtin_bswap64" => Some(CType::ULongLong),

            // __builtin_expect returns long (evaluates both args, returns first)
            "__builtin_expect" | "__builtin_expect_with_probability" => Some(CType::Long),

            // Complex component extraction
            "creal" | "__builtin_creal" | "cimag" | "__builtin_cimag" => Some(CType::Double),
            "crealf" | "__builtin_crealf" | "cimagf" | "__builtin_cimagf" => Some(CType::Float),
            "creall" | "__builtin_creall" | "cimagl" | "__builtin_cimagl" => {
                Some(CType::LongDouble)
            }
            "cabs" | "__builtin_cabs" => Some(CType::Double),
            "cabsf" | "__builtin_cabsf" => Some(CType::Float),
            "cabsl" | "__builtin_cabsl" => Some(CType::LongDouble),
            "carg" | "__builtin_carg" => Some(CType::Double),
            "cargf" | "__builtin_cargf" => Some(CType::Float),

            // Memory/string builtins returning pointers
            "__builtin_memcpy"
            | "__builtin_memmove"
            | "__builtin_memset"
            | "__builtin_alloca"
            | "__builtin_alloca_with_align"
            | "alloca"
            | "__builtin_frame_address"
            | "__builtin_return_address"
            | "__builtin_thread_pointer" => {
                Some(CType::Pointer(Box::new(CType::Void), PtrQuals::GENERIC))
            }

            // Void-returning builtins
            "__builtin_va_start"
            | "__builtin_va_end"
            | "__builtin_va_copy"
            | "__builtin_abort"
            | "__builtin_exit"
            | "__builtin_trap"
            | "__builtin_unreachable"
            | "__builtin_prefetch"
            | "__builtin___clear_cache"
            | "__builtin_cpu_init" => Some(CType::Void),

            // CPU feature detection
            "__builtin_cpu_supports" | "__builtin_cpu_is" => Some(CType::Int),

            _ => None,
        }
    }

    /// Resolve the type of a variable declared within a compound statement.
    ///
    /// Scans declarations in order, building a local scope so that typeof
    /// expressions referencing earlier declarations can be resolved. This handles
    /// patterns like: `({ typeof(&s->field) p = ...; __typeof__(*p) ret = ...; ret; })`
    /// where `__typeof__(*p)` must resolve `p` from the same compound statement.
    fn resolve_var_from_compound(
        &self,
        compound: &CompoundStmt,
        target_name: &str,
        parent_scope: Option<&FxHashMap<String, CType>>,
    ) -> Option<CType> {
        let mut local_scope: FxHashMap<String, CType> = FxHashMap::default();

        // Seed with the enclosing statement expression's scope so nested
        // compounds (kernel xchg/cmpxchg macros: outer declares __ai_ptr,
        // inner declares __ret as typeof(*__ai_ptr)) can resolve types that
        // reference declarations from an OUTER compound.
        if let Some(parent) = parent_scope {
            for (k, v) in parent {
                local_scope.insert(k.clone(), v.clone());
            }
        }

        for item in &compound.items {
            if let BlockItem::Declaration(decl) = item {
                for declarator in &decl.declarators {
                    if declarator.name.is_empty() {
                        continue;
                    }
                    // Resolve this declaration's type, using the local scope
                    // for typeof expressions referencing earlier declarations.
                    let base = self.resolve_type_spec_with_scope(&decl.type_spec, &local_scope);
                    let mut ctype = crate::common::type_builder::build_full_ctype_with_callbacks(
                        base,
                        &declarator.derived,
                        &|ts| self.resolve_type_spec_with_scope(ts, &local_scope),
                        &|e, _| self.eval_const_expr(e).map(|v| v as usize),
                    );
                    crate::common::type_builder::apply_declaration_address_space_for_object(
                        &mut ctype,
                        decl.address_space,
                        &declarator.derived,
                    );
                    if declarator.name == target_name {
                        return Some(ctype);
                    }
                    local_scope.insert(declarator.name.clone(), ctype);
                }
            }
        }
        None
    }

    /// Build the declaration scope of a compound statement without resolving
    /// a specific target: every declaration's type is resolved against the
    /// accumulated local scope (seeded from an optional parent scope), then
    /// inserted.  Used to thread enclosing statement-expression scopes into
    /// nested statement-expression type inference.
    fn build_compound_scope(
        &self,
        compound: &CompoundStmt,
        parent_scope: Option<&FxHashMap<String, CType>>,
    ) -> FxHashMap<String, CType> {
        let mut local_scope: FxHashMap<String, CType> = FxHashMap::default();
        if let Some(parent) = parent_scope {
            for (k, v) in parent {
                local_scope.insert(k.clone(), v.clone());
            }
        }
        for item in &compound.items {
            if let BlockItem::Declaration(decl) = item {
                for declarator in &decl.declarators {
                    if declarator.name.is_empty() {
                        continue;
                    }
                    let base = self.resolve_type_spec_with_scope(&decl.type_spec, &local_scope);
                    let mut ctype = crate::common::type_builder::build_full_ctype_with_callbacks(
                        base,
                        &declarator.derived,
                        &|ts| self.resolve_type_spec_with_scope(ts, &local_scope),
                        &|e, _| self.eval_const_expr(e).map(|v| v as usize),
                    );
                    crate::common::type_builder::apply_declaration_address_space_for_object(
                        &mut ctype,
                        decl.address_space,
                        &declarator.derived,
                    );
                    local_scope.insert(declarator.name.clone(), ctype);
                }
            }
        }
        local_scope
    }

    /// Infer the type of a statement expression whose identifiers may not be
    /// in the symbol table yet, threading an optional parent scope from an
    /// enclosing statement expression (see infer_expr_ctype's StmtExpr arm).
    fn infer_stmt_expr_ctype_with_scope(
        &self,
        compound: &CompoundStmt,
        parent_scope: Option<&FxHashMap<String, CType>>,
    ) -> Option<CType> {
        let Some(expr) = compound.value_expr() else {
            // Mirrors `infer_expr_ctype`: a tail that cannot produce a value
            // makes the whole statement expression `void`.
            return Some(CType::Void);
        };
        // Nested statement expression: build this compound's scope and
        // recurse with it as the parent so inner typeof()s can see variables
        // declared here.  Do this before plain inference for the same reason
        // as infer_expr_ctype's StmtExpr arm: plain inference of an unexecuted
        // nested stmt-expr can return a wrong fallback type instead of None.
        if let Expr::StmtExpr(inner_compound, _) = expr {
            let scope = self.build_compound_scope(compound, parent_scope);
            if !scope.is_empty()
                && let Some(ctype) =
                    self.infer_stmt_expr_ctype_with_scope(inner_compound, Some(&scope))
            {
                return Some(ctype);
            }
        }
        if let Some(ctype) = self.infer_expr_ctype(expr) {
            return Some(ctype);
        }
        // Plain identifier: resolve from declarations in this compound
        // (including those inherited from enclosing compounds).
        if let Expr::Identifier(name, _) = expr {
            return self.resolve_var_from_compound(compound, name, parent_scope);
        }
        None
    }

    /// Resolve a TypeSpecifier to a CType, using a supplementary local scope
    /// for typeof expressions that reference variables not in the symbol table.
    fn resolve_type_spec_with_scope(
        &self,
        ts: &TypeSpecifier,
        scope: &FxHashMap<String, CType>,
    ) -> CType {
        match ts {
            TypeSpecifier::Typeof(expr, _) => {
                // Try normal resolution first
                if let Some(ctype) = self.infer_expr_ctype(expr) {
                    return ctype;
                }
                // Try resolving using the local scope
                if let Some(ctype) = self.infer_expr_ctype_with_scope(expr, scope) {
                    return ctype;
                }
                // TODO: report diagnostic for unresolved typeof expression
                CType::Int // fallback
            }
            TypeSpecifier::TypeofType(inner, _) => self.resolve_type_spec_with_scope(inner, scope),
            _ => self.resolve_type_spec(ts),
        }
    }

    /// Infer the CType of an expression using a supplementary local scope
    /// for identifiers not found in the symbol table.
    /// Handles common typeof patterns (identifier, deref, address-of).
    /// More complex expressions (member access, subscript, etc.) are not supported.
    /// Address space of the object an lvalue designates: the declared space of
    /// a variable, the pointee space of a `->` or `*` operand, and the space of
    /// the enclosing object for `.` and `[]` on an array. Anything else (a call,
    /// a temporary, a generic object) is `Default`.
    pub fn lvalue_object_space(&self, expr: &Expr) -> AddressSpace {
        match expr {
            Expr::Identifier(name, _) => self
                .symbols
                .lookup(name)
                .map_or(AddressSpace::Default, |sym| sym.address_space),
            Expr::MemberAccess(base, _, _) => self.lvalue_object_space(base),
            Expr::PointerMemberAccess(base, _, _) | Expr::Deref(base, _) => self
                .operand_memory_space(base)
                .unwrap_or(AddressSpace::Default),
            // `a[i]` and the reversed `i[a]` walk into whichever operand is the
            // pointer or array (C defines both as `*(a + i)`).
            Expr::ArraySubscript(base, index, _) => self
                .operand_memory_space(base)
                .or_else(|| self.operand_memory_space(index))
                .unwrap_or(AddressSpace::Default),
            _ => AddressSpace::Default,
        }
    }

    /// Space of the memory a pointer or array operand points into. `None` for
    /// an operand that is neither (an integer index).
    fn operand_memory_space(&self, operand: &Expr) -> Option<AddressSpace> {
        match self.infer_expr_ctype(operand)? {
            CType::Pointer(_, space) => Some(space.space),
            CType::Array(_, _) => Some(self.lvalue_object_space(operand)),
            _ => None,
        }
    }

    fn infer_expr_ctype_with_scope(
        &self,
        expr: &Expr,
        scope: &FxHashMap<String, CType>,
    ) -> Option<CType> {
        match expr {
            Expr::Identifier(name, _) => scope.get(name.as_str()).cloned(),
            Expr::Deref(inner, _) => {
                let inner_ct = self
                    .infer_expr_ctype(inner)
                    .or_else(|| self.infer_expr_ctype_with_scope(inner, scope))?;
                match inner_ct {
                    CType::Pointer(pointee, _) => Some(*pointee),
                    CType::Array(elem, _) => Some(*elem),
                    _ => None,
                }
            }
            Expr::AddressOf(inner, _) => {
                let inner_ct = self
                    .infer_expr_ctype(inner)
                    .or_else(|| self.infer_expr_ctype_with_scope(inner, scope))?;
                let space = self.lvalue_address_space(inner);
                Some(CType::Pointer(Box::new(inner_ct), PtrQuals::of(space)))
            }
            // Statement expression: thread the current scope down so nested
            // typeof() patterns (kernel xchg/cmpxchg macros) resolve.
            Expr::StmtExpr(compound, _) => {
                self.infer_stmt_expr_ctype_with_scope(compound, Some(scope))
            }
            _ => None,
        }
    }

    /// Space that `typeof(expr)` as a pointer base contributes; see
    /// `type_builder::TypeConvertContext::typeof_pointee_space`.
    fn typeof_pointee_space(&self, leaf: &TypeSpecifier) -> AddressSpace {
        match leaf {
            // typeof_unqual drops the named address space: no space.
            TypeSpecifier::Typeof(expr, false) => self.lvalue_address_space(expr),
            _ => AddressSpace::Default,
        }
    }

    /// Named address space of the storage an lvalue designates — the property
    /// `&E` must carry into its pointer type (C11 6.5.3.2p3).
    ///
    /// This is sema's mirror of the lowerer's `get_addr_space_of_struct_expr`
    /// (src/ir/lowering/pointer_analysis.rs), deliberately walking the same
    /// shapes: an identifier gets its space from the declaration record, a
    /// member or subscript expression from its base object, and `p->m` / `*p`
    /// from the space of the pointer `p` points with. Sema and lowering must
    /// agree — lowering already prefixes the load (kernel per-CPU objects read
    /// through `%gs:`), so a generic sema type is not a cosmetic mismatch: it
    /// rejects correct kernel code such as `__tasklet_schedule_common(t,
    /// &tasklet_vec, TASKLET_SOFTIRQ)` whose parameter is `struct tasklet_head
    /// __percpu *`.
    pub(super) fn lvalue_address_space(&self, expr: &Expr) -> AddressSpace {
        match expr {
            Expr::Identifier(name, _) => {
                // `__func__` and enum constants are not objects with storage.
                self.symbols
                    .lookup(name)
                    .map(|sym| sym.address_space)
                    .unwrap_or(AddressSpace::Default)
            }
            // `s.inner.field` lives in the base object's space.
            Expr::MemberAccess(base, _, _) => self.lvalue_address_space(base),
            // `arr[i]` lives in the array's space, but `p[i]` for a pointer `p`
            // dereferences `p`: the element lives in the space `p` points into,
            // not in the space of the pointer variable (which is generic).
            // GCC 14.2: `int __seg_gs *gp; &gp[1]` is a `__seg_gs` pointer.
            Expr::ArraySubscript(base, _, _) => match self.infer_expr_ctype(base) {
                Some(CType::Pointer(_, _)) => self.pointee_address_space(base),
                _ => self.lvalue_address_space(base),
            },
            // `p->field` / `(*p).field`: the member lives in the space p points
            // into, which may come from p's type rather than its declaration.
            Expr::PointerMemberAccess(base, _, _) | Expr::Deref(base, _) => {
                self.pointee_address_space(base)
            }
            // `*(__seg_gs T *)n` and friends.
            Expr::Cast(TypeSpecifier::Pointer(_, space), _, _, _) => *space,
            _ => AddressSpace::Default,
        }
    }

    /// Named address space of the object a pointer expression designates.
    /// Prefers the expression's type (a `__seg_gs T *` value carries its space
    /// there) and otherwise looks through unqualified casts to `&obj`, matching
    /// `get_addr_space_of_ptr_expr` in the lowerer.
    fn pointee_address_space(&self, expr: &Expr) -> AddressSpace {
        if let Some(CType::Pointer(_, quals)) = self.infer_expr_ctype(expr) {
            if quals.space != AddressSpace::Default {
                return quals.space;
            }
        }
        match expr {
            Expr::Cast(TypeSpecifier::Pointer(_, space), inner, _, _) => {
                if *space != AddressSpace::Default {
                    *space
                } else {
                    self.pointee_address_space(inner)
                }
            }
            Expr::Cast(_, inner, _, _) => self.pointee_address_space(inner),
            Expr::AddressOf(inner, _) => self.lvalue_address_space(inner),
            _ => AddressSpace::Default,
        }
    }

    fn infer_c2y_builtin_ctype(&self, name: &str, args: &[Expr]) -> Option<CType> {
        use crate::frontend::sema::{gcc16_builtins, stdbit};
        if stdbit::is_stdc_bit_builtin(name) {
            let arg0_ct = args
                .first()
                .and_then(|a| self.infer_expr_ctype(a))
                .map(|ct| ct.decayed_value_ctype());
            return stdbit::stdc_bit_return_ctype(name, arg0_ct.as_ref());
        }
        if gcc16_builtins::is_gcc16_extra_builtin(name) {
            let arg0_ct = args
                .first()
                .and_then(|a| self.infer_expr_ctype(a))
                .map(|ct| ct.decayed_value_ctype());
            return gcc16_builtins::gcc16_builtin_return_ctype(name, arg0_ct.as_ref());
        }
        match name {
            "__builtin_counted_by_ref" => {
                if let Some(arg0) = args.first() {
                    if let Some((cnt_ty, space)) = self.infer_counted_by_target(arg0) {
                        return Some(CType::Pointer(Box::new(cnt_ty), PtrQuals::of(space)));
                    }
                }
                Some(CType::Pointer(Box::new(CType::Void), PtrQuals::GENERIC))
            }
            "__builtin_assoc_barrier" => args
                .first()
                .and_then(|a| self.infer_expr_ctype(a))
                .map(|ct| ct.decayed_value_ctype()),
            "__builtin_clear_padding" => Some(CType::Void),
            "__builtin_object_size" | "__builtin_dynamic_object_size" => {
                Some(if crate::common::types::target_is_32bit() {
                    CType::UInt
                } else {
                    CType::ULong
                })
            }
            _ => None,
        }
    }

    pub(crate) fn infer_counted_by_target(&self, expr: &Expr) -> Option<(CType, AddressSpace)> {
        let (base_expr, field_name, is_ptr) = match expr {
            Expr::MemberAccess(b, f, _) => (b.as_ref(), f.as_str(), false),
            Expr::PointerMemberAccess(b, f, _) => (b.as_ref(), f.as_str(), true),
            _ => return None,
        };
        let base_ct = self.infer_expr_ctype(base_expr)?;
        let (record_ct, space) = if is_ptr {
            match base_ct.decayed_value_ctype() {
                CType::Pointer(inner, quals) => (*inner, quals.space),
                _ => return None,
            }
        } else {
            (base_ct, self.lvalue_address_space(base_expr))
        };
        let key = match &record_ct {
            CType::Struct(k) => k,
            _ => return None,
        };
        let layouts = self.types.borrow_struct_layouts();
        let layout = layouts.get(&**key)?;
        let (_, cnt_ty, _) = layout.field_counted_by(field_name, &*layouts)?;
        Some((cnt_ty, space))
    }
}

#[cfg(test)]
mod qualifier_composition_tests {
    use super::*;
    use crate::common::source::Span;
    use crate::common::symbol_table::Symbol;

    #[test]
    fn cached_and_uncached_compositions_keep_identical_qualifier_records() {
        let mut symbols = SymbolTable::new();
        symbols.declare(Symbol {
            name: "a".into(),
            ty: CType::Array(Box::new(CType::Int), Some(2)),
            explicit_alignment: None,
            type_align: None,
            address_space: AddressSpace::Default,
        });
        symbols.mark_object_cvr("a", Some(PtrQuals::CONST));
        let types = TypeContext::new();
        let functions = FxHashMap::default();
        let checker = ExprTypeChecker {
            symbols: &symbols,
            types: &types,
            functions: &functions,
            expr_types: None,
            builtin_policy: Default::default(),
            pre_c99: false,
        };
        let span = Span::new(0, 1, 0);
        let array = Expr::Identifier("a".into(), span);
        let comma = Expr::Comma(Box::new(Expr::IntLiteral(0, span)), Box::new(array), span);
        let address = Expr::AddressOf(Box::new(Expr::Identifier("a".into(), span)), span);
        let conditional = Expr::Conditional(
            Box::new(Expr::IntLiteral(1, span)),
            Box::new(address.clone()),
            Box::new(address),
            span,
        );
        for expr in [&comma, &conditional] {
            let mut cache = FxHashMap::default();
            expr.for_each_child(&mut |child| {
                cache.insert(child.id(), checker.infer_expr_ctype(child).unwrap());
            });
            let cached = ExprTypeChecker {
                expr_types: Some(&cache),
                builtin_policy: Default::default(),
                ..checker
            };
            let a = checker.infer_value_ctype(expr).unwrap();
            let b = cached.infer_value_ctype(expr).unwrap();
            let (CType::Pointer(_, aq), CType::Pointer(_, bq)) = (a, b) else {
                panic!("pointer expected")
            };
            assert_eq!(aq.cvr(), Some(PtrQuals::CONST));
            assert_eq!(aq.cvr(), bq.cvr());
        }
    }
}
