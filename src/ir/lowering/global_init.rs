//! Global initialization subsystem for the IR lowerer.
//!
//! Lowers AST `Initializer` nodes to IR `GlobalInit` values. Covers:
//! - Scalar initializers (integers, floats, long doubles, booleans)
//! - String literals (narrow, wide, char16) in both array and pointer contexts
//! - Compound literals at file scope (`&(struct S){1, 2}`)
//! - Array initializers (1D, multi-dimensional, designated, pointer arrays)
//! - Struct/union initializers (byte-serialized or compound/relocation-aware)
//! - Complex type arrays (`_Complex float arr[]`)
//! - Vector type arrays (`int __attribute__((vector_size(16))) arr[]`)
//!
//! The top-level entry point `lower_global_init` dispatches to focused helpers
//! for each initializer category, keeping each function short and readable.

use crate::frontend::parser::ast::{
    BinOp, Designator, Expr, Initializer, InitializerItem, TypeSpecifier, UnaryOp,
};
use crate::ir::reexports::{GlobalInit, IrConst, IrGlobal};

/// An operand of a folded global relational expression (`eval_global_relational`):
/// an integer constant, or the address of a named global plus a byte offset.
enum GlobalOperand {
    Int(i64),
    Addr(String, i64),
}

/// Kind of string literal for extract_string_literal helper.
enum StringLitKind {
    Narrow,
    Wide,
    Char16,
}
use super::array_init_plan::ArrayGeometry;
use super::global_init_helpers as h;
use super::lower::Lowerer;
use super::vector_array_init::VectorArrayInitPlan;
use crate::common::types::{CType, IrType, RcLayout, StructLayout};

// =============================================================================
// Top-level entry point
// =============================================================================

impl Lowerer {
    /// Lower a global initializer to a GlobalInit value.
    ///
    /// This is the main dispatch: expressions go to `lower_global_init_expr`,
    /// initializer lists go to `lower_global_init_list`.
    pub(super) fn lower_global_init(
        &mut self,
        init: &Initializer,
        type_spec: &TypeSpecifier,
        base_ty: IrType,
        is_array: bool,
        elem_size: usize,
        total_size: usize,
        struct_layout: &Option<RcLayout>,
        array_dim_strides: &[usize],
        obj_ty: Option<&CType>,
    ) -> GlobalInit {
        // Every scalar leaf of the initializer converts to the object's
        // innermost element type, taken from the object's C type itself (an
        // array typedef or `typeof` names the array, not the element).
        let elem_ct = self.init_leaf_ctype(obj_ty, type_spec);
        let elem_ct = &elem_ct;

        // An object of array type initialized by a compound literal of that
        // same type is the object initialized by the literal's payload list:
        // the literal names an array object (C11 6.5.2.5p5) and an array
        // cannot be initialized from an array *expression* (6.7.9p13), so the
        // two spellings must agree.  Synthesizing the list here keeps every
        // array flavour -- scalars of any rank, character arrays, arrays of
        // aggregates -- on the paths below, canonicalisation included.
        let synthesized: Initializer;
        let init: &Initializer = match obj_ty.and_then(|t| self.array_initializer_payload(init, t))
        {
            Some(payload) => {
                synthesized = Initializer::List(payload.to_vec());
                &synthesized
            }
            None => init,
        };

        // GCC's `folding_initializer`: see `Lowerer::folding_initializer`.
        let outer = self.folding_initializer.replace(true);
        let lowered = match init {
            Initializer::Expr(expr) => {
                self.lower_global_init_expr(expr, base_ty, is_array, obj_ty, elem_ct)
            }
            Initializer::List(items) => {
                // Resolve brace elision, designator chains and overrides of
                // struct/union and array-of-aggregate initializers once
                // (`init_canon`); canonical lists are lowered as written.
                let canonical = obj_ty.and_then(|t| {
                    let t = self.complete_array_ctype(t, total_size);
                    self.canonical_init_items(items, &t, None)
                });
                let items = canonical.as_deref().unwrap_or(items);
                self.lower_global_init_list(
                    items,
                    base_ty,
                    is_array,
                    elem_size,
                    total_size,
                    struct_layout,
                    array_dim_strides,
                    elem_ct,
                    obj_ty,
                )
            }
        };
        self.folding_initializer.set(outer);
        lowered
    }

    /// The innermost non-array C type of the object being initialized: the
    /// type every scalar leaf of its initializer converts to. Taken from the
    /// object's C type when known (an array typedef or `typeof` names the
    /// whole array), otherwise from the declaration's type specifier.
    pub(super) fn init_leaf_ctype(
        &self,
        obj_ty: Option<&CType>,
        type_spec: &TypeSpecifier,
    ) -> CType {
        let mut ct = obj_ty
            .cloned()
            .unwrap_or_else(|| self.type_spec_to_ctype(type_spec));
        while let CType::Array(inner, _) = ct {
            ct = *inner;
        }
        ct
    }
}

// =============================================================================
// Expression initializers (Initializer::Expr)
// =============================================================================

impl Lowerer {
    /// Lower an expression initializer: scalar constants, string literals,
    /// compound literals, address expressions, and label differences.
    fn lower_global_init_expr(
        &mut self,
        expr: &Expr,
        base_ty: IrType,
        is_array: bool,
        obj_ty: Option<&CType>,
        elem_ct: &CType,
    ) -> GlobalInit {
        // Resolve _Generic selections: unwrap to the selected expression before
        // any other processing, so all downstream logic (const eval, string
        // literals, address expressions, etc.) sees the resolved expression.
        if let Expr::GenericSelection(controlling, associations, _) = expr {
            if let Some(selected) = self.resolve_generic_selection_expr(controlling, associations) {
                return self.lower_global_init_expr(selected, base_ty, is_array, obj_ty, elem_ct);
            }
        }

        // Complex types: handle before scalar evaluation to prevent misinterpretation
        // of scalar values (e.g., `_Complex float g = 1.0f;`).
        //
        // Every type decision below uses `elem_ct`, the object's own scalar
        // type -- never `type_spec`, the declaration's base specifier: for
        // `_Decimal64 *p = (_Decimal64 *)64;` the base is `_Decimal64` but
        // the object is a pointer, and converting the address as a decimal
        // stored its BID encoding (likewise `long double *`, `_Bool *`).
        if elem_ct.is_complex() {
            if let Some(init) = self.eval_complex_global_init(expr, elem_ct) {
                return init;
            }
        }
        // A vector from a constant vector expression (`v = (T){ ... }`). The
        // object's own type decides: the declaration's type specifier is only its
        // base specifier (`T (*fp)(int) = f;` has base type `T`).
        if let Some(vec_ct) = obj_ty.filter(|t| t.is_vector()) {
            let mut bytes = vec![0u8; self.resolve_ctype_size(vec_ct)];
            self.write_vector_expr_bytes(expr, vec_ct, &mut bytes, 0);
            return GlobalInit::Array(bytes.iter().map(|&b| IrConst::I8(b as i8)).collect());
        }

        // Scalar constant
        //
        // A *bare* read of a folded const object is answered from sema's map
        // here: `const int cc = 5; int v = cc;` is a constant initializer
        // under GCC's folding rule (C11 6.7.9p4 would want an integer constant
        // expression), and GCC rejects the same read of a non-const or
        // volatile object.  `eval_const_expr` deliberately declines
        // identifiers -- it resolves them by name so that a lowering-local
        // scope wins -- which is why `cc * 2` was folded and `cc` was not, and
        // why the read was diagnosed as non-constant (measured: GCC accepts
        // all six scalar shapes in tests/regression/file_scope_init/values.c).
        // Sema answers exactly GCC's rule: `annotate_const_value` records the
        // value under the read's expression id, and `declares_const_object`
        // refuses to record a volatile object (C11 6.7.3p7).
        // sema's recorded value is the *contract*: every initializer this
        // compiler accepts has a constant answer recorded under the
        // initializer's own expression id, including the shapes the lowering
        // evaluator declines on purpose (`eval_const_expr` resolves identifiers
        // by name so a lowering-local scope wins) and the ones it cannot
        // express (a constant folded out of a call or out of an element of a
        // folded const object).  The lowering recomputes when it can -- its own
        // answer reflects lowering-local scoping -- and otherwise consumes
        // sema's, which is the answer the program was accepted on.  `sizeof` is
        // the one exclusion: sema evaluates it against the *declared* type,
        // before the lowerer has resolved unsized array dimensions from the
        // initializer, so an expression containing one is recomputed here.
        let scalar = self.eval_const_expr(expr).or_else(|| {
            if Self::expr_contains_sizeof(expr) {
                None
            } else {
                self.lookup_sema_const(expr)
            }
        });
        if let Some(val) = scalar {
            // C23 decimals: the initializer must become exact BID bits via
            // the master const conversion (integers convert by value with
            // half-even rounding, binary floats through their exact
            // expansion, BID sources by decode/re-encode) — never a raw
            // integer/float bit-cast (`_Decimal32 g = 5` used to emit the
            // I64 {5}). Unhandled kinds fall through to coerce (status quo).
            let target_ct = elem_ct.clone();
            if target_ct.is_decimal() {
                let width = match target_ct {
                    CType::Decimal32 => 32,
                    CType::Decimal64 => 64,
                    _ => 128,
                };
                if let Some(src_ct) = self.get_expr_ctype(expr) {
                    if let Some(bid) =
                        crate::common::const_arith::const_to_bid(&val, &src_ct, width)
                    {
                        return GlobalInit::Scalar(bid);
                    }
                } else if !matches!(val, IrConst::I128(_)) {
                    // Unknown source kind, unambiguous carrier: signedness
                    // from the const-type query (literals and enums always
                    // know their type; the I128 carrier never guesses).
                    let fallback = if self.is_expr_unsigned_for_const(expr) {
                        CType::UInt
                    } else {
                        CType::Int
                    };
                    if let Some(bid) =
                        crate::common::const_arith::const_to_bid(&val, &fallback, width)
                    {
                        return GlobalInit::Scalar(bid);
                    }
                }
            }
            // _Float128 targets and _Float128 / long double / decimal sources
            // convert by value (`coerce_init_const`), never by carrier bits.
            return GlobalInit::Scalar(self.coerce_scalar_const(
                val,
                expr,
                base_ty,
                Some(&target_ct),
            ));
        }

        // String literal (narrow, wide, or char16)
        if let Some(init) = self.lower_string_literal_init(expr, base_ty, is_array) {
            return init;
        }

        // `__func__`/`__FUNCTION__`/`__PRETTY_FUNCTION__` denote a static
        // const char[] per function (C99 6.4.2.2p4), so a static local
        // initialized with one is an address constant (GCC 16.2 accepts
        // `static const char *s = __func__;`; the kernel's tracepoint string
        // helpers — lib/maple_tree.c trace_ma_op — do exactly that). Material
        // the function name as the same interned string literal the
        // expression path lowers it to.
        // At file scope it is GCC's string for that spelling
        // (`function_name_builtin_text`).
        if let Expr::Identifier(name, _) = expr {
            if crate::frontend::parser::ast::is_function_name_builtin(name) {
                let text = crate::frontend::parser::ast::function_name_builtin_text(
                    name,
                    self.func_state.as_ref().map(|fs| fs.source_name()),
                )
                .to_string();
                return GlobalInit::GlobalAddr(self.intern_string_literal(&text));
            }
        }

        // &(compound_literal) at file scope
        if let Expr::AddressOf(inner, _) = expr {
            if let Expr::CompoundLiteral(cl_type_spec, cl_init, _, cl_st) = inner.as_ref() {
                return self.create_compound_literal_global_with_storage(
                    cl_type_spec,
                    cl_init,
                    cl_st,
                );
            }
        }

        // Cast-wrapped compound literal: e.g., (char *)(unsigned char[]){ 0xFD }
        // Also handles (void*) &(CompoundLiteral) pattern used in static initializers.
        {
            let stripped = Self::strip_casts(expr);
            if !std::ptr::eq(expr as *const _, stripped as *const _) {
                if let Expr::CompoundLiteral(cl_type_spec, cl_init, _, cl_st) = stripped {
                    return self.create_compound_literal_global_with_storage(
                        cl_type_spec,
                        cl_init,
                        cl_st,
                    );
                }
                // (void*) &(CompoundLiteral) – cast wrapping address-of compound literal
                if let Expr::AddressOf(inner, _) = stripped {
                    if let Expr::CompoundLiteral(cl_type_spec, cl_init, _, cl_st) = inner.as_ref() {
                        return self.create_compound_literal_global_with_storage(
                            cl_type_spec,
                            cl_init,
                            cl_st,
                        );
                    }
                }
            }
        }

        // Compound literal used directly as initializer value
        if let Expr::CompoundLiteral(cl_type_spec, cl_init, _, _) = expr {
            return self.lower_compound_literal_init(cl_type_spec, cl_init, base_ty, is_array);
        }

        // C23 constant aggregate copy (e.g. `constexpr struct Pair g2 = g1;`):
        if let Expr::Identifier(src_name, _) = expr {
            if let Some(img) = self.const_object_images.get(src_name.as_str()) {
                if matches!(img.ctype, CType::Struct(..) | CType::Union(..)) {
                    let vals = img.bytes.iter().map(|&b| IrConst::I8(b as i8)).collect();
                    return GlobalInit::Array(vals);
                }
            }
        }

        // String literal with offset: "str" + N or "str" - N
        if let Some(addr_init) = self.eval_string_literal_addr_expr(expr) {
            return addr_init;
        }

        // Pre-materialize compound literals embedded in address arithmetic.
        // This handles patterns like:
        //   (type*)((char*)(int[]){1,2,3} + 8)
        // where a compound literal is used as the base of pointer arithmetic.
        // We create anonymous globals for such compound literals so that
        // eval_global_addr_expr can resolve them as GlobalAddr references.
        self.materialize_compound_literals_in_expr(expr);

        // Global address expression: &x, func, arr, &arr[3], &s.field
        if let Some(addr_init) = self.eval_global_addr_expr(expr) {
            return addr_init;
        }

        // Label difference: &&lab1 - &&lab2 (computed goto dispatch tables)
        if let Some(label_diff) = self.eval_label_diff_expr(expr, base_ty.size().max(4)) {
            return label_diff;
        }

        // Relational, equality and logical operators over address constants,
        // and characters of string literals. GCC folds these in a static
        // initializer, and sema accepts exactly these forms (static_init.rs),
        // so each is folded here instead of falling through to the
        // non-constant diagnostic below.
        if let Some(v) = self.eval_global_relational(expr) {
            return GlobalInit::Scalar(self.coerce_scalar_const(
                IrConst::I64(v),
                expr,
                base_ty,
                Some(elem_ct),
            ));
        }

        // Every value-preserving form above failed, so this initialiser is not
        // a constant expression.  C requires a constant expression here
        // (C11 6.6/6.7.9), and the old fallback silently stored zero —
        // `static int v = f();` became `v = 0` with no diagnostic while GCC,
        // Clang and ICX all report "initializer element is not constant"
        // (measured, including for `__builtin_cpu_supports`, which GCC refuses
        // to fold in constant contexts).  Diagnose instead of miscompiling.
        self.diagnose_non_constant(expr);
        GlobalInit::Zero
    }

    /// Report a static initializer element no constant form could lower.
    /// Every lowering path ends here instead of storing zero: sema accepts
    /// what GCC accepts, so reaching this is either a non-constant or a form
    /// the lowering lacks, and both must be an error, never a silent 0.
    pub(super) fn diagnose_non_constant(&self, expr: &Expr) {
        self.diagnostics
            .borrow_mut()
            .error("initializer element is not constant", expr.span());
    }

    /// The operand of a folded global relational or logical expression: an
    /// integer constant, or the address of a named global with a byte offset.
    fn global_operand(&self, expr: &Expr) -> Option<GlobalOperand> {
        // __builtin_choose_expr selects one operand at compile time; the
        // choice can be an ADDRESS constant (the kernel's
        // PIN_GROUP(.modes = __builtin_choose_expr(
        // __builtin_constant_p((m)), (const void *)0, (m))) names the static
        // pin array in the unselected-by-constant_p arm), so the selection
        // must consult the address path, not just the value path.
        if let Expr::FunctionCall(callee, args, _) = expr {
            if matches!(callee.as_ref(), Expr::Identifier(name, _) if name == "__builtin_choose_expr")
                && args.len() >= 3
            {
                let chosen = match self.eval_const_expr(&args[0]) {
                    Some(v) if v.is_nonzero() => &args[1],
                    Some(_) => &args[2],
                    None => return None,
                };
                return self.global_operand(chosen);
            }
        }
        if let Some(v) = self.eval_const_expr(expr).and_then(|c| c.to_i64()) {
            return Some(GlobalOperand::Int(v));
        }
        match self.eval_global_addr_expr(expr)? {
            GlobalInit::GlobalAddr(name) => Some(GlobalOperand::Addr(name, 0)),
            GlobalInit::GlobalAddrOffset(name, off) => Some(GlobalOperand::Addr(name, off)),
            _ => None,
        }
    }

    /// The truth value of a folded operand: an address is never null.
    fn global_truth(&self, expr: &Expr) -> Option<bool> {
        if self.is_string_literal_addr_expr(expr) {
            return Some(true);
        }
        match self.global_operand(expr) {
            Some(GlobalOperand::Int(v)) => Some(v != 0),
            Some(GlobalOperand::Addr(..)) => Some(true),
            // A nested relation or logical form: `!(&g == 0)`,
            // `(&g != 0) && (&h != 0)` (GCC folds both).  Each call descends
            // into a strict subexpression, so the recursion terminates.
            None => self.eval_global_relational(expr).map(|v| v != 0),
        }
    }

    /// The value of a relational, equality or logical expression over address
    /// constants, or of a character of an ASCII string literal, as an integer.
    /// `None` when the form is not one of these (the caller then decides).
    fn eval_global_relational(&self, expr: &Expr) -> Option<i64> {
        match expr {
            Expr::ArraySubscript(base, index, _) => {
                let Expr::StringLiteral(text, _) = base.as_ref() else {
                    return None;
                };
                if !text.is_ascii() {
                    return None;
                }
                let i = usize::try_from(self.eval_const_expr(index)?.to_i64()?).ok()?;
                // The character at `len` is the terminating NUL.
                text.as_bytes()
                    .get(i)
                    .map(|&b| i64::from(b))
                    .or_else(|| (i == text.len()).then_some(0))
            }
            Expr::UnaryOp(UnaryOp::LogicalNot, inner, _) => {
                Some(i64::from(!self.global_truth(inner)?))
            }
            Expr::BinaryOp(BinOp::LogicalAnd, lhs, rhs, _) => Some(i64::from(
                self.global_truth(lhs)? && self.global_truth(rhs)?,
            )),
            Expr::BinaryOp(BinOp::LogicalOr, lhs, rhs, _) => Some(i64::from(
                self.global_truth(lhs)? || self.global_truth(rhs)?,
            )),
            Expr::BinaryOp(op, lhs, rhs, _) => self.eval_global_compare(*op, lhs, rhs),
            // `(T)(&g != 0)`: GCC folds the cast of such a value too.  Every
            // conversion to an integer, `_Bool`, pointer, binary or decimal
            // floating type maps 0 to 0 and 1 to 1 exactly (GCC's integer to
            // pointer conversion keeps the value), so the cast is peeled only
            // for those targets and only for a 0/1 operand; a string
            // character outside {0, 1} or a vector/complex/aggregate target
            // still declines.
            Expr::Cast(ts, inner, _, _) => {
                let to = self.type_spec_to_ctype(ts);
                let exact = to.is_integer()
                    || to.is_floating()
                    || to.is_soft_float()
                    || matches!(to, CType::Pointer(..));
                if !exact {
                    return None;
                }
                self.eval_global_relational(inner)
                    .filter(|v| matches!(v, 0 | 1))
            }
            _ => None,
        }
    }

    /// A comparison of two global operands. Distinct objects are never equal
    /// (GCC folds `&g == &h` to 0), an address is greater than the null
    /// constant, and ordering across distinct objects is not a constant (GCC
    /// rejects `&g < &h`), so it yields `None`.
    fn eval_global_compare(&self, op: BinOp, lhs: &Expr, rhs: &Expr) -> Option<i64> {
        use std::cmp::Ordering;
        let (a, b) = (self.global_operand(lhs)?, self.global_operand(rhs)?);
        let equal = match (&a, &b) {
            (GlobalOperand::Addr(x, ox), GlobalOperand::Addr(y, oy)) => Some(x == y && ox == oy),
            (GlobalOperand::Addr(..), GlobalOperand::Int(0))
            | (GlobalOperand::Int(0), GlobalOperand::Addr(..)) => Some(false),
            _ => None,
        };
        let order = match (&a, &b) {
            (GlobalOperand::Addr(x, ox), GlobalOperand::Addr(y, oy)) if x == y => Some(ox.cmp(oy)),
            (GlobalOperand::Addr(..), GlobalOperand::Int(0)) => Some(Ordering::Greater),
            (GlobalOperand::Int(0), GlobalOperand::Addr(..)) => Some(Ordering::Less),
            _ => None,
        };
        match op {
            BinOp::Eq => equal.map(i64::from),
            BinOp::Ne => equal.map(|e| i64::from(!e)),
            BinOp::Lt => order.map(|o| i64::from(o.is_lt())),
            BinOp::Le => order.map(|o| i64::from(o.is_le())),
            BinOp::Gt => order.map(|o| i64::from(o.is_gt())),
            BinOp::Ge => order.map(|o| i64::from(o.is_ge())),
            _ => None,
        }
    }

    /// Recursively walk an expression tree and materialize any compound literals
    /// found within arithmetic/cast contexts as anonymous globals. This is needed
    /// so that `eval_global_addr_expr` (which takes `&self`) can resolve compound
    /// literals as `GlobalAddr` references when they appear as bases of pointer
    /// arithmetic in global initializers.
    ///
    /// For example, the Cello library uses:
    ///   `var Alloc = (var)((char*)((var[]){ NULL, ... }) + sizeof(struct Header))`
    /// The compound literal `(var[]){ NULL, ... }` must be materialized as an
    /// anonymous global before the pointer arithmetic can be resolved.
    pub(super) fn materialize_compound_literals_in_expr(&mut self, expr: &Expr) {
        match expr {
            Expr::CompoundLiteral(type_spec, init, _, st) => {
                let key = expr as *const Expr as usize;
                if !self.materialized_compound_literals.contains_key(&key) {
                    // Materialize this compound literal as an anonymous global
                    let init_result =
                        self.create_compound_literal_global_with_storage(type_spec, init, st);
                    // Extract the global name from the result
                    if let GlobalInit::GlobalAddr(label) = init_result {
                        self.materialized_compound_literals.insert(key, label);
                    }
                }
            }
            Expr::Cast(_, inner, _, _) => {
                self.materialize_compound_literals_in_expr(inner);
            }
            Expr::BinaryOp(_, lhs, rhs, _) => {
                self.materialize_compound_literals_in_expr(lhs);
                self.materialize_compound_literals_in_expr(rhs);
            }
            Expr::UnaryOp(_, inner, _) => {
                self.materialize_compound_literals_in_expr(inner);
            }
            Expr::AddressOf(inner, _) => {
                self.materialize_compound_literals_in_expr(inner);
            }
            Expr::Conditional(cond, then_e, else_e, _) => {
                self.materialize_compound_literals_in_expr(cond);
                self.materialize_compound_literals_in_expr(then_e);
                self.materialize_compound_literals_in_expr(else_e);
            }
            _ => {}
        }
    }

    /// Try to resolve an expression as `&(compound_literal)`, possibly wrapped
    /// in casts. If found, materializes the compound literal as an anonymous global
    /// and returns `GlobalInit::GlobalAddr(label)`.
    ///
    /// This is used by `collect_compound_init_element` to handle `&((struct T){...})`
    /// patterns inside compound literal arrays (e.g., Cello's Instance() macro).
    /// Note: `lower_global_init_expr` has similar logic at lines 106-110 for the
    /// top-level expression case; this function handles the array element case.
    fn try_address_of_compound_literal(&mut self, expr: &Expr) -> Option<GlobalInit> {
        let stripped = Self::strip_casts(expr);
        if let Expr::AddressOf(inner, _) = stripped {
            if let Expr::CompoundLiteral(cl_type_spec, cl_init, _, cl_st) = inner.as_ref() {
                return Some(self.create_compound_literal_global_with_storage(
                    cl_type_spec,
                    cl_init,
                    cl_st,
                ));
            }
        }
        None
    }

    /// Coerce a scalar constant to the target type, handling bool normalization
    /// and long double promotion.
    fn coerce_scalar_const(
        &self,
        val: IrConst,
        expr: &Expr,
        base_ty: IrType,
        target_ct: Option<&CType>,
    ) -> IrConst {
        let src_ty = self.get_expr_type(expr);
        let val = self.coerce_init_const(val, expr, base_ty, target_ct);
        if target_ct == Some(&CType::LongDouble) {
            Self::promote_to_long_double_with_signedness(val, src_ty)
        } else {
            val
        }
    }

    /// Promote a constant to long double, respecting signedness for integer sources.
    fn promote_to_long_double_with_signedness(val: IrConst, src_ty: IrType) -> IrConst {
        match val {
            IrConst::F64(v) => IrConst::long_double(v),
            IrConst::F32(v) => IrConst::long_double(v as f64),
            IrConst::I64(v) => {
                if src_ty.is_unsigned() {
                    IrConst::long_double_from_u64(v as u64)
                } else {
                    IrConst::long_double_from_i64(v)
                }
            }
            IrConst::I32(v) => IrConst::long_double_from_i64(v as i64),
            other => other,
        }
    }

    /// Lower a string literal expression to GlobalInit.
    ///
    /// Handles all three string literal kinds (narrow, wide, char16) in a single
    /// path. Returns None if the expression is not a string literal.
    fn lower_string_literal_init(
        &mut self,
        expr: &Expr,
        base_ty: IrType,
        is_array: bool,
    ) -> Option<GlobalInit> {
        let (s, kind) = self.extract_string_literal(expr)?;
        Some(self.string_literal_to_global_init(&s, kind, base_ty, is_array))
    }

    /// Convert a string literal to a GlobalInit based on the target type context.
    ///
    /// When initializing an array, the string is inlined (as bytes, u32s, or u16s
    /// depending on the element type). When initializing a pointer, the string is
    /// interned in .rodata and referenced by label.
    fn string_literal_to_global_init(
        &mut self,
        s: &str,
        kind: StringLitKind,
        base_ty: IrType,
        is_array: bool,
    ) -> GlobalInit {
        if is_array {
            // Inline the string into the array based on the target element type
            match base_ty {
                IrType::I8 | IrType::U8 => GlobalInit::String(s.to_string()),
                IrType::I32 | IrType::U32 => {
                    GlobalInit::WideString(s.chars().map(|c| c as u32).collect())
                }
                IrType::I16 | IrType::U16 => GlobalInit::Char16String(s.encode_utf16().collect()),
                _ => {
                    // Unexpected element type for string init - treat as pointer
                    self.intern_string_as_global_addr(s, kind)
                }
            }
        } else {
            // Pointer context: intern in .rodata and return address
            self.intern_string_as_global_addr(s, kind)
        }
    }

    /// Intern a string literal and return a GlobalAddr referencing it.
    fn intern_string_as_global_addr(&mut self, s: &str, kind: StringLitKind) -> GlobalInit {
        let label = match kind {
            StringLitKind::Narrow => self.intern_string_literal(s),
            StringLitKind::Wide => self.intern_wide_string_literal(s),
            StringLitKind::Char16 => self.intern_char16_string_literal(s),
        };
        GlobalInit::GlobalAddr(label)
    }

    /// Lower a compound literal used directly as an initializer value.
    fn lower_compound_literal_init(
        &mut self,
        cl_type_spec: &TypeSpecifier,
        cl_init: &Initializer,
        _base_ty: IrType,
        is_array: bool,
    ) -> GlobalInit {
        let cl_ctype = self.type_spec_to_ctype(cl_type_spec);
        let is_aggregate = matches!(
            cl_ctype,
            CType::Struct(..) | CType::Union(..) | CType::Array(..)
        );

        if !is_aggregate {
            // Scalar or pointer compound literal: create anonymous global
            return self.create_compound_literal_global(cl_type_spec, cl_init);
        }

        // When a compound literal of array type initializes a non-array variable,
        // it's array-to-pointer decay: create an anonymous global for the array
        // and use its address as the initializer value.
        let cl_is_array_type = matches!(cl_ctype, CType::Array(..));
        if cl_is_array_type && !is_array {
            return self.create_compound_literal_global(cl_type_spec, cl_init);
        }

        // Aggregate compound literal: recursively lower using its own type info.
        let cl_base_ty = self.type_spec_to_ir(cl_type_spec);
        let cl_layout = self.get_struct_layout_for_type(cl_type_spec);
        let cl_size = cl_layout
            .as_ref()
            .map_or_else(|| self.sizeof_type(cl_type_spec), |l| l.size);
        let cl_is_array = matches!(cl_ctype, CType::Array(..));
        self.lower_global_init(
            cl_init,
            cl_type_spec,
            cl_base_ty,
            cl_is_array,
            0,
            cl_size,
            &cl_layout,
            &[],
            Some(&cl_ctype),
        )
    }
}

// =============================================================================
// List initializers (Initializer::List)
// =============================================================================

impl Lowerer {
    /// Lower an initializer list. Dispatches to specialized handlers based on
    /// the target type: brace-wrapped strings, complex arrays, struct arrays,
    /// pointer arrays, vector arrays, flat/multidim arrays, structs, vectors,
    /// and scalar-with-braces.
    fn lower_global_init_list(
        &mut self,
        items: &[InitializerItem],
        base_ty: IrType,
        is_array: bool,
        elem_size: usize,
        total_size: usize,
        struct_layout: &Option<RcLayout>,
        array_dim_strides: &[usize],
        elem_ct: &CType,
        obj_ty: Option<&CType>,
    ) -> GlobalInit {
        // Brace-wrapped string literal: char c[] = {"hello"}
        let is_char_not_ptr_array = elem_size <= base_ty.size().max(1);
        if is_array && is_char_not_ptr_array {
            if let Some(init) = self.try_brace_wrapped_string(items, base_ty) {
                return init;
            }
        }

        // Complex array: double _Complex arr[] = { ... }.  The leaf type
        // decides, not the base specifier: `_Complex double *a[2]` is an
        // array of pointers (it was lowered as complex values).
        if is_array && elem_ct.is_complex() {
            return self.lower_complex_array_init(items, elem_ct, total_size, elem_size);
        }

        // Arrays of ext-vectors have vector objects as their leaf
        // subobjects. Resolve their dimensions/designators independently of the
        // pointer-sized IR carrier, then serialize each selected vector to its
        // C object representation.
        if is_array {
            if let Some(array_type) = obj_ty.map(|ty| self.complete_array_ctype(ty, total_size)) {
                if let Some(plan) = self.plan_vector_array_init(items, &array_type) {
                    return self.lower_vector_array_plan_to_global_init(plan, total_size);
                }
            }
        }

        // Array of scalars of any rank: resolved by the array planner.
        if is_array {
            if let Some(geo) = obj_ty
                .map(|t| self.complete_array_ctype(t, total_size))
                .and_then(|t| self.scalar_array_geometry(&t))
            {
                return self.lower_scalar_array_plan(items, &geo, total_size);
            }
        }

        // Array with elements
        if is_array && elem_size > 0 {
            // Array of vectors (`v4f a[] = ...`). The object's type decides:
            // an array of pointers to vectors has base specifier `v4f` too.
            if let Some(CType::Array(elem, _)) = obj_ty {
                if elem.is_vector() {
                    let len = total_size / self.resolve_ctype_size(elem).max(1);
                    return self.lower_vector_array_init(items, elem, len);
                }
            }
            return self.lower_array_init(
                items,
                base_ty,
                elem_size,
                total_size,
                struct_layout,
                array_dim_strides,
                elem_ct,
            );
        }

        // Struct/union initializer list
        if let Some(layout) = struct_layout {
            return self.lower_struct_global_init(items, layout);
        }

        // Vector initializer list: __attribute__((vector_size(N))). The
        // object's type decides, not the base specifier (`T *p = { &v };`).
        if let Some(vec_ct) = obj_ty.filter(|t| t.is_vector()) {
            return self.lower_vector_init(items, vec_ct);
        }

        // Scalar with braces: int x = { 1 }; or int x = {{{1}}};
        if !is_array && !items.is_empty() {
            if let Some(init) = self.lower_scalar_with_braces(items, base_ty, obj_ty) {
                return init;
            }
        }

        // Fallback: array of constants coerced to base_ty
        let elem_ct = obj_ty.and_then(|t| match t {
            CType::Array(elem, _) => Some(elem.as_ref()),
            _ => Some(t),
        });
        self.lower_fallback_array(items, base_ty, elem_ct)
    }

    /// Serialize the selected vector-object initializers into a byte array.
    /// Each vector starts at its C object stride; omitted vectors and all
    /// per-vector padding remain zero from the initial image.
    fn lower_vector_array_plan_to_global_init(
        &self,
        plan: VectorArrayInitPlan,
        total_size: usize,
    ) -> GlobalInit {
        let mut bytes = vec![0u8; total_size];
        let vector_size = plan.vector_type.size();
        for (index, init) in plan.elements.into_iter().enumerate() {
            let Some(init) = init else { continue };
            let offset = index.saturating_mul(vector_size);
            self.write_vector_init_bytes(&init, &plan.vector_type, &mut bytes, offset);
        }
        GlobalInit::Array(
            bytes
                .into_iter()
                .map(|byte| IrConst::I8(byte as i8))
                .collect(),
        )
    }

    /// Try to extract a brace-wrapped string literal: `{"hello"}` for char/wchar_t/char16_t arrays.
    fn try_brace_wrapped_string(
        &mut self,
        items: &[InitializerItem],
        base_ty: IrType,
    ) -> Option<GlobalInit> {
        if items.len() != 1 || !items[0].designators.is_empty() {
            return None;
        }
        let expr = match &items[0].init {
            Initializer::Expr(e) => e,
            _ => return None,
        };
        let (s, _kind) = self.extract_string_literal(expr)?;
        // Convert string to appropriate GlobalInit based on element type
        Some(match base_ty {
            IrType::I8 | IrType::U8 => GlobalInit::String(s),
            IrType::I32 | IrType::U32 => {
                GlobalInit::WideString(s.chars().map(|c| c as u32).collect())
            }
            IrType::I16 | IrType::U16 => GlobalInit::Char16String(s.encode_utf16().collect()),
            _ => return None,
        })
    }

    /// Lower a scalar value wrapped in braces: `int x = { 1 };` or `int x = {{{1}}}`.
    fn lower_scalar_with_braces(
        &mut self,
        items: &[InitializerItem],
        base_ty: IrType,
        dst_ct: Option<&CType>,
    ) -> Option<GlobalInit> {
        let expr = Self::unwrap_nested_init_expr(items)?;
        if let Some(val) = self.eval_const_expr(expr) {
            return Some(GlobalInit::Scalar(
                self.coerce_init_const(val, expr, base_ty, dst_ct),
            ));
        }
        self.eval_global_addr_expr(expr)
    }

    /// Fallback: emit all items as an array of constants.
    fn lower_fallback_array(
        &self,
        items: &[InitializerItem],
        base_ty: IrType,
        elem_ct: Option<&CType>,
    ) -> GlobalInit {
        let values: Vec<IrConst> = items
            .iter()
            .map(|item| {
                if let Initializer::Expr(expr) = &item.init {
                    self.eval_const_expr(expr)
                        .map(|v| self.coerce_init_const(v, expr, base_ty, elem_ct))
                        .unwrap_or_else(|| {
                            self.diagnose_non_constant(expr);
                            self.zero_const(base_ty)
                        })
                } else {
                    self.zero_const(base_ty)
                }
            })
            .collect();
        if values.is_empty() {
            GlobalInit::Zero
        } else {
            GlobalInit::Array(values)
        }
    }
}

// =============================================================================
// Array initializers
// =============================================================================

impl Lowerer {
    /// Lower an array initializer list. Dispatches based on element type:
    /// struct arrays, pointer arrays with relocations, multi-dimensional
    /// arrays, and flat arrays (arrays of vectors are dispatched by the
    /// caller, which knows the object's type).
    fn lower_array_init(
        &mut self,
        items: &[InitializerItem],
        base_ty: IrType,
        elem_size: usize,
        total_size: usize,
        struct_layout: &Option<RcLayout>,
        array_dim_strides: &[usize],
        elem_ct: &CType,
    ) -> GlobalInit {
        let num_elems = self.compute_num_elems(
            base_ty,
            elem_size,
            total_size,
            struct_layout,
            *elem_ct == CType::LongDouble,
        );

        // Struct array (byte-serialized or compound for pointer fields)
        if let Some(layout) = struct_layout {
            return self.lower_struct_array_init(
                items,
                layout,
                num_elems,
                total_size,
                array_dim_strides,
            );
        }

        // Pointer array or array with address expressions (needs .quad directives)
        if self.array_needs_compound_init(items, base_ty, elem_size, array_dim_strides) {
            return self.lower_pointer_array_init(
                items,
                base_ty,
                elem_size,
                total_size,
                array_dim_strides,
            );
        }

        // Plain scalar array (possibly multi-dimensional)
        self.lower_scalar_array_init(
            items,
            base_ty,
            num_elems,
            total_size,
            array_dim_strides,
            elem_ct,
        )
    }

    /// Compute the number of elements in the array.
    fn compute_num_elems(
        &self,
        base_ty: IrType,
        elem_size: usize,
        total_size: usize,
        struct_layout: &Option<RcLayout>,
        is_long_double_target: bool,
    ) -> usize {
        if struct_layout.is_some() || is_long_double_target {
            total_size / elem_size.max(1)
        } else {
            let base_type_size = base_ty.size().max(1);
            if elem_size > base_type_size {
                total_size / elem_size
            } else {
                total_size / base_type_size
            }
        }
    }

    /// Lower a struct array initializer. Uses byte-serialization for structs
    /// without pointer fields, and compound representation for structs with pointers.
    fn lower_struct_array_init(
        &mut self,
        items: &[InitializerItem],
        layout: &StructLayout,
        num_elems: usize,
        total_size: usize,
        array_dim_strides: &[usize],
    ) -> GlobalInit {
        // Which image can express this array: bytes only, or bytes plus
        // pointer-sized relocations?  The element *type* is not the whole
        // answer.  `struct x86_cpu_id { ... unsigned long driver_data; }` has no
        // pointer member, yet the kernel's CPU tables initialize that member
        // with an address constant -- `{ .driver_data = (unsigned long)&model }`
        // (arch/x86/events/rapl.c, the X86_MATCH_* tables).  A byte image
        // cannot hold a relocation, so the content decides, exactly as it does
        // for a single struct object (`struct_init_has_addr_fields`); the
        // declared type alone routed these arrays to the byte serializer, which
        // can only report "initializer element is not constant".
        // Does the image need relocations?  A pointer member forces it; so does
        // an address constant stored in a pointer-sized integer member, which
        // is exactly how the kernel's `x86_cpu_id` tables are written
        // (`(unsigned long)&model`).  Those tables have no pointer member at
        // all, so the declared type alone used to send them to the byte
        // serializer, which cannot hold a relocation and could only report
        // "initializer element is not constant" (GCC, Clang and ICX accept
        // them: C11 6.6p9).  Both halves are needed -- the member that can hold
        // an address, and an initializer that stores one -- so a byte image is
        // chosen only when one of them is absent.
        if self.init_needs_relocations(items, layout) {
            if array_dim_strides.len() > 1 {
                return self.lower_struct_array_with_ptrs_multidim(
                    items,
                    layout,
                    total_size,
                    array_dim_strides,
                );
            }
            return self.lower_struct_array_with_ptrs(items, layout, num_elems);
        }
        // Byte-serialize the struct array
        let struct_size = layout.size;
        let mut bytes = vec![0u8; total_size];
        self.fill_multidim_struct_array_bytes(
            items,
            layout,
            struct_size,
            array_dim_strides,
            &mut bytes,
            0,
            total_size,
        );
        let values: Vec<IrConst> = bytes.iter().map(|&b| IrConst::I8(b as i8)).collect();
        GlobalInit::Array(values)
    }

    /// Check if an array initializer needs compound (relocation-aware) representation.
    /// This is needed when elements contain address expressions, string literals used
    /// as pointers, or label addresses.
    fn array_needs_compound_init(
        &self,
        items: &[InitializerItem],
        base_ty: IrType,
        elem_size: usize,
        array_dim_strides: &[usize],
    ) -> bool {
        let is_multidim_char_array =
            matches!(base_ty, IrType::I8 | IrType::U8) && array_dim_strides.len() > 1;
        let is_ptr_array = elem_size > base_ty.size().max(1);

        let has_addr_exprs = items.iter().any(|item| {
            if let Initializer::Expr(expr) = &item.init {
                // Resolve _Generic to its selected expression for detection
                let expr = if let Expr::GenericSelection(controlling, associations, _) = expr {
                    if let Some(selected) =
                        self.resolve_generic_selection_expr(controlling, associations)
                    {
                        selected
                    } else {
                        expr
                    }
                } else {
                    expr
                };
                if matches!(expr, Expr::StringLiteral(_, _)) {
                    return !is_multidim_char_array;
                }
                if matches!(Self::strip_casts(expr), Expr::LabelAddr(_, _))
                    || Self::expr_contains_label_addr(expr)
                {
                    return true;
                }
                let is_const = self.eval_const_expr(expr).is_some();
                if !is_const && self.eval_global_addr_expr(expr).is_some() {
                    return true;
                }
                if is_const {
                    return false;
                }
            }
            h::init_contains_addr_expr(item, is_multidim_char_array, &self.types.enum_constants)
        });

        has_addr_exprs
            || (is_ptr_array
                && !is_multidim_char_array
                && items
                    .iter()
                    .any(|item| h::init_contains_string_literal(item)))
    }

    /// A static array of scalars from its braced initializer list: a
    /// constant list, or per-element `GlobalInit`s when an element is an
    /// address.
    fn lower_scalar_array_plan(
        &mut self,
        items: &[InitializerItem],
        geo: &ArrayGeometry,
        total_size: usize,
    ) -> GlobalInit {
        let plan = self.plan_array_init(items, geo);
        if !self.plan_needs_relocations(&plan, geo) {
            return GlobalInit::Array(self.plan_to_consts(&plan, geo));
        }
        let elements = self.plan_to_global_elems(&plan, geo);
        // Where a pointer is narrower than the element (`uint64_t` holding an
        // address on i686), the element's upper bytes are explicit zeros.
        if geo.elem_size() > crate::common::types::target_ptr_size() {
            return self.convert_compound_to_bytes_and_ptrs(elements, geo.elem_size(), total_size);
        }
        GlobalInit::Compound(elements)
    }

    /// Lower a pointer array or array with address relocations.
    /// Each element becomes a separate GlobalInit in a Compound representation.
    fn lower_pointer_array_init(
        &mut self,
        items: &[InitializerItem],
        base_ty: IrType,
        elem_size: usize,
        total_size: usize,
        array_dim_strides: &[usize],
    ) -> GlobalInit {
        let base_type_size = base_ty.size().max(1);
        let flat_num_elems = total_size / base_type_size;
        let outer_stride = if array_dim_strides.len() >= 2 {
            array_dim_strides[0] / base_type_size
        } else {
            1
        };
        let mut elements: Vec<GlobalInit> = (0..flat_num_elems).map(|_| GlobalInit::Zero).collect();
        let is_structured = outer_stride > 1
            && items
                .iter()
                .any(|item| matches!(&item.init, Initializer::List(_)));

        let mut current_idx = 0usize;
        for item in items {
            let index_designators: Vec<usize> = item
                .designators
                .iter()
                .filter_map(|d| {
                    if let Designator::Index(idx_expr) = d {
                        self.eval_const_expr(idx_expr).and_then(|c| c.to_usize())
                    } else {
                        None
                    }
                })
                .collect();

            if !index_designators.is_empty() {
                current_idx = index_designators[0];
            }

            let flat_idx = if index_designators.len() > 1 {
                self.compute_flat_index_from_designators(
                    &index_designators,
                    array_dim_strides,
                    elem_size.max(base_ty.size().max(1)),
                )
            } else if is_structured {
                current_idx * outer_stride
            } else {
                current_idx
            };

            if flat_idx < flat_num_elems {
                // For inner List initializers, handle designators to place
                // elements at the correct offset within the sub-array.
                if let Initializer::List(sub_items) = &item.init {
                    if is_structured {
                        // Process inner list with designator support.
                        // Compute how many flat elements each inner sub-item spans.
                        // For arr[2][2][3] with strides [48,24,8] and base_type_size=8,
                        // inner_stride = 24/8 = 3. For 2D arrays, inner_stride = 1.
                        let inner_stride = if array_dim_strides.len() >= 2 {
                            array_dim_strides[1] / base_type_size
                        } else {
                            1
                        };
                        let mut inner_idx = 0usize;
                        for sub in sub_items {
                            // Check for inner designator
                            let inner_desig: Vec<usize> = sub
                                .designators
                                .iter()
                                .filter_map(|d| {
                                    if let Designator::Index(idx_expr) = d {
                                        self.eval_const_expr(idx_expr).and_then(|c| c.to_usize())
                                    } else {
                                        None
                                    }
                                })
                                .collect();
                            if !inner_desig.is_empty() {
                                inner_idx = inner_desig[0];
                            }
                            let target_idx = flat_idx + inner_idx * inner_stride;
                            if target_idx < flat_num_elems {
                                let mut elem_parts = Vec::new();
                                self.collect_compound_init_element(
                                    &sub.init,
                                    &mut elem_parts,
                                    elem_size,
                                );
                                for (i, elem) in elem_parts.into_iter().enumerate() {
                                    if target_idx + i < flat_num_elems {
                                        let elem = match elem {
                                            GlobalInit::Scalar(val) => {
                                                GlobalInit::Scalar(val.coerce_to(base_ty))
                                            }
                                            other => other,
                                        };
                                        elements[target_idx + i] = elem;
                                    }
                                }
                            }
                            inner_idx += 1;
                        }
                    } else {
                        let mut elem_parts = Vec::new();
                        self.collect_compound_init_element(&item.init, &mut elem_parts, elem_size);
                        for (i, elem) in elem_parts.into_iter().enumerate() {
                            if flat_idx + i < flat_num_elems {
                                let elem = match elem {
                                    GlobalInit::Scalar(val) => {
                                        GlobalInit::Scalar(val.coerce_to(base_ty))
                                    }
                                    other => other,
                                };
                                elements[flat_idx + i] = elem;
                            }
                        }
                    }
                } else {
                    let mut elem_parts = Vec::new();
                    self.collect_compound_init_element(&item.init, &mut elem_parts, elem_size);
                    for (i, elem) in elem_parts.into_iter().enumerate() {
                        if flat_idx + i < flat_num_elems {
                            let elem = match elem {
                                GlobalInit::Scalar(val) => {
                                    GlobalInit::Scalar(val.coerce_to(base_ty))
                                }
                                other => other,
                            };
                            elements[flat_idx + i] = elem;
                        }
                    }
                }
            }
            current_idx += 1;
        }

        // On targets where pointer size < element type size (e.g., i686 with
        // uint64_t[]), convert to byte+pointer representation for correct padding.
        let ptr_size = crate::common::types::target_ptr_size();
        if base_type_size > ptr_size {
            return self.convert_compound_to_bytes_and_ptrs(elements, base_type_size, total_size);
        }
        GlobalInit::Compound(elements)
    }

    /// Convert compound elements to byte+pointer representation for targets where
    /// pointer size < element size.
    fn convert_compound_to_bytes_and_ptrs(
        &self,
        elements: Vec<GlobalInit>,
        base_type_size: usize,
        total_size: usize,
    ) -> GlobalInit {
        let mut bytes = vec![0u8; total_size];
        let mut ptr_ranges: Vec<(usize, GlobalInit)> = Vec::new();
        let ptr_size = crate::common::types::target_ptr_size();
        for (idx, elem) in elements.into_iter().enumerate() {
            let byte_offset = idx * base_type_size;
            match elem {
                GlobalInit::Zero => {}
                GlobalInit::Scalar(ref c) => {
                    let val_bytes = c.to_le_bytes();
                    let len = val_bytes.len().min(base_type_size);
                    bytes[byte_offset..byte_offset + len].copy_from_slice(&val_bytes[..len]);
                }
                GlobalInit::GlobalAddr(_) | GlobalInit::GlobalAddrOffset(_, _) => {
                    ptr_ranges.push((byte_offset, elem));
                }
                GlobalInit::GlobalLabelDiff(lab1, lab2, _) => {
                    ptr_ranges.push((
                        byte_offset,
                        GlobalInit::GlobalLabelDiff(lab1, lab2, ptr_size),
                    ));
                }
                _ => {}
            }
        }
        Self::build_compound_from_bytes_and_ptrs(bytes, ptr_ranges, total_size)
    }

    /// Lower an array of complex types: `_Complex double arr[] = { ... }`.
    fn lower_complex_array_init(
        &mut self,
        items: &[InitializerItem],
        complex_ctype: &CType,
        total_size: usize,
        elem_size: usize,
    ) -> GlobalInit {
        let num_elems = total_size / elem_size.max(1);
        let total_scalars = num_elems * 2;
        let zero_pair = Self::complex_zero_pair(complex_ctype);
        let mut values: Vec<IrConst> = Vec::with_capacity(total_scalars);
        for _ in 0..num_elems {
            values.extend_from_slice(&zero_pair);
        }
        let mut current_idx = 0usize;
        for item in items {
            if let Some(Designator::Index(idx_expr)) = item.designators.first() {
                if let Some(idx) = self.eval_const_expr(idx_expr).and_then(|c| c.to_usize()) {
                    current_idx = idx;
                }
            }
            if current_idx < num_elems {
                let expr = match &item.init {
                    Initializer::Expr(e) => Some(e),
                    Initializer::List(sub_items) => Self::unwrap_nested_init_expr(sub_items),
                };
                if let Some(expr) = expr {
                    if let Some((real, imag)) = self.eval_complex_const_public(expr) {
                        let base_offset = current_idx * 2;
                        let (r, i) = Self::complex_pair(complex_ctype, real, imag);
                        values[base_offset] = r;
                        values[base_offset + 1] = i;
                    }
                }
            }
            current_idx += 1;
        }
        GlobalInit::Array(values)
    }

    /// Get the zero pair for a complex type.
    fn complex_zero_pair(ctype: &CType) -> Vec<IrConst> {
        match ctype {
            CType::ComplexFloat => vec![IrConst::F32(0.0), IrConst::F32(0.0)],
            CType::ComplexLongDouble => vec![IrConst::long_double(0.0), IrConst::long_double(0.0)],
            _ => vec![IrConst::F64(0.0), IrConst::F64(0.0)],
        }
    }

    /// Create a (real, imag) pair for a complex type.
    fn complex_pair(ctype: &CType, real: f64, imag: f64) -> (IrConst, IrConst) {
        match ctype {
            CType::ComplexFloat => (IrConst::F32(real as f32), IrConst::F32(imag as f32)),
            CType::ComplexLongDouble => (IrConst::long_double(real), IrConst::long_double(imag)),
            _ => (IrConst::F64(real), IrConst::F64(imag)),
        }
    }

    /// Lower a global array of vectors (`v4f a[N] = { ... }`) to its byte
    /// image.
    fn lower_vector_array_init(
        &self,
        items: &[InitializerItem],
        vec_ct: &CType,
        num_elems: usize,
    ) -> GlobalInit {
        let vec_size = self.resolve_ctype_size(vec_ct);
        let mut bytes = vec![0u8; vec_size * num_elems];
        self.fill_array_of_vectors(items, vec_ct, num_elems, &mut bytes, 0);
        GlobalInit::Array(bytes.iter().map(|&b| IrConst::I8(b as i8)).collect())
    }

    /// Byte image of an array of `len` vectors at `offset`: element `i` takes
    /// the vector initializer at position `i` (positional or `[i]`-designated).
    pub(super) fn fill_array_of_vectors(
        &self,
        items: &[InitializerItem],
        vec_ct: &CType,
        len: usize,
        bytes: &mut [u8],
        offset: usize,
    ) {
        let vec_size = self.resolve_ctype_size(vec_ct);
        let mut pos = 0usize;
        for item in items {
            if let Some(Designator::Index(idx)) = item.designators.first() {
                match self.eval_const_expr(idx).and_then(|c| c.to_usize()) {
                    Some(i) => pos = i,
                    None => {
                        self.diagnose_non_constant(idx);
                        continue;
                    }
                }
            }
            if pos < len {
                self.write_vector_init_bytes(&item.init, vec_ct, bytes, offset + pos * vec_size);
            }
            pos += 1;
        }
    }

    /// Write the byte image of a vector initializer at `offset`: a braced
    /// list of lanes (`[i]`-designated or positional; unmentioned lanes stay
    /// zero) or a constant vector expression.
    pub(super) fn write_vector_init_bytes(
        &self,
        init: &Initializer,
        vec_ct: &CType,
        bytes: &mut [u8],
        offset: usize,
    ) {
        match init {
            Initializer::List(items) => self.write_vector_lanes(items, vec_ct, bytes, offset),
            Initializer::Expr(e) => self.write_vector_expr_bytes(e, vec_ct, bytes, offset),
        }
    }

    fn write_vector_lanes(
        &self,
        items: &[InitializerItem],
        vec_ct: &CType,
        bytes: &mut [u8],
        offset: usize,
    ) {
        let Some((elem_ct, lanes)) = vec_ct.vector_info() else {
            return;
        };
        let elem_ir_ty = IrType::from_ctype(elem_ct);
        // Byte stride of one lane: the element size for `vector_size`, the
        // ext-vector stride (x87 lanes are 10 bytes apart); `None`
        // for a bit-packed `_Bool` ext-vector, whose lane `i` is bit `i % 8`
        // of byte `i / 8`.
        let lane_stride = vec_ct.vector_element_storage_size();
        let mut pos = 0usize;
        for item in items {
            if let Some(Designator::Index(idx)) = item.designators.first() {
                match self.eval_const_expr(idx).and_then(|c| c.to_usize()) {
                    Some(i) => pos = i,
                    None => {
                        self.diagnose_non_constant(idx);
                        continue;
                    }
                }
            }
            // A braced scalar (`{ 1 }`) initializes one lane.
            let mut init = &item.init;
            while let Initializer::List(inner) = init {
                match inner.first() {
                    Some(first) => init = &first.init,
                    None => break,
                }
            }
            if let (true, Initializer::Expr(e)) = (pos < lanes, init) {
                let val = self.eval_const_expr(e).unwrap_or_else(|| {
                    self.diagnose_non_constant(e);
                    IrConst::I64(0)
                });
                let val = self.coerce_init_const(val, e, elem_ir_ty, Some(elem_ct));
                match lane_stride {
                    Some(stride) if vec_ct.is_x87_lane_vector() => {
                        // x87 lanes are 10 bytes apart: write the 16-byte
                        // F128 image to a scratch and copy only the 80-bit
                        // payload, so a designated out-of-order lane does
                        // not zero the first six bytes of its neighbour.
                        let mut image = [0u8; 16];
                        self.write_const_to_bytes(&mut image, 0, &val, elem_ir_ty);
                        let at = offset + pos * stride;
                        for (i, b) in image.iter().take(stride).enumerate() {
                            if let Some(dst) = bytes.get_mut(at + i) {
                                *dst = *b;
                            }
                        }
                    }
                    Some(stride) => {
                        self.write_const_to_bytes(bytes, offset + pos * stride, &val, elem_ir_ty);
                    }
                    None => {
                        if let Some(byte) = bytes.get_mut(offset + pos / 8) {
                            let bit = 1u8 << (pos % 8);
                            if val.is_nonzero() {
                                *byte |= bit;
                            } else {
                                *byte &= !bit;
                            }
                        }
                    }
                }
            }
            pos += 1;
        }
    }

    /// A constant vector expression (GCC 16.2's set): a compound literal of
    /// the vector's type, a cast of one to that same type, or a cast of an
    /// integer constant of the vector's size, which supplies its bits. Sema
    /// rejects the rest (casts across vector types, vector arithmetic).
    fn write_vector_expr_bytes(&self, e: &Expr, vec_ct: &CType, bytes: &mut [u8], offset: usize) {
        let size = self.resolve_ctype_size(vec_ct);
        match e {
            Expr::CompoundLiteral(ts, init, _, _) if self.type_spec_to_ctype(ts) == *vec_ct => {
                self.write_vector_init_bytes(init, vec_ct, bytes, offset);
                return;
            }
            Expr::Cast(ts, inner, _, _) if self.type_spec_to_ctype(ts) == *vec_ct => {
                match self.get_expr_ctype(inner) {
                    Some(from) if from == *vec_ct => {
                        self.write_vector_expr_bytes(inner, vec_ct, bytes, offset);
                        return;
                    }
                    Some(from) if from.is_integer() && self.resolve_ctype_size(&from) == size => {
                        if let Some(val) = self.eval_const_expr(inner) {
                            self.write_const_to_bytes(
                                bytes,
                                offset,
                                &val,
                                IrType::from_ctype(&from),
                            );
                            return;
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
        self.diagnose_non_constant(e);
    }

    /// Lower a plain scalar array (1D or multi-dimensional).
    fn lower_scalar_array_init(
        &mut self,
        items: &[InitializerItem],
        base_ty: IrType,
        num_elems: usize,
        total_size: usize,
        array_dim_strides: &[usize],
        elem_ct: &CType,
    ) -> GlobalInit {
        let is_long_double_target = *elem_ct == CType::LongDouble;
        let zero_val = self.typed_zero_const(base_ty, is_long_double_target);

        if array_dim_strides.len() > 1 {
            // Multi-dimensional array: flatten nested init lists
            let innermost_stride = array_dim_strides.last().copied().unwrap_or(1).max(1);
            let total_scalar_elems = total_size / innermost_stride;
            let mut values_flat =
                vec![self.typed_zero_const(base_ty, is_long_double_target); total_scalar_elems];
            let mut flat = Vec::with_capacity(total_scalar_elems);
            self.flatten_global_array_init_bool(
                items,
                array_dim_strides,
                base_ty,
                &mut flat,
                elem_ct,
            );
            for (i, v) in flat.into_iter().enumerate() {
                if i < total_scalar_elems {
                    values_flat[i] = Self::maybe_promote_long_double(v, is_long_double_target);
                }
            }
            GlobalInit::Array(values_flat)
        } else {
            // 1D array with designated initializer support
            let mut values = vec![zero_val; num_elems];
            let mut current_idx = 0usize;
            for item in items {
                if let Some(Designator::Index(idx_expr)) = item.designators.first() {
                    if let Some(idx) = self.eval_const_expr(idx_expr).and_then(|c| c.to_usize()) {
                        current_idx = idx;
                    }
                }
                if current_idx < num_elems {
                    let val = self.eval_array_element(&item.init, base_ty, elem_ct);
                    values[current_idx] =
                        Self::maybe_promote_long_double(val, is_long_double_target);
                }
                current_idx += 1;
            }
            GlobalInit::Array(values)
        }
    }

    /// Evaluate a single array element from its initializer.
    fn eval_array_element(&self, init: &Initializer, base_ty: IrType, elem_ct: &CType) -> IrConst {
        match init {
            Initializer::Expr(expr) => {
                let raw = self
                    .eval_const_expr(expr)
                    .unwrap_or(self.zero_const(base_ty));
                self.coerce_init_const(raw, expr, base_ty, Some(elem_ct))
            }
            // A brace-enclosed scalar (`{{2.75Q}}`): its first leaf, converted
            // like any other element.
            Initializer::List(sub_items) => sub_items.first().map_or_else(
                || self.zero_const(base_ty),
                |first| self.eval_array_element(&first.init, base_ty, elem_ct),
            ),
        }
    }

    /// Lower a global vector's braced initializer to its byte image.
    fn lower_vector_init(&self, items: &[InitializerItem], vec_ct: &CType) -> GlobalInit {
        let mut bytes = vec![0u8; self.resolve_ctype_size(vec_ct)];
        self.write_vector_lanes(items, vec_ct, &mut bytes, 0);
        GlobalInit::Array(bytes.iter().map(|&b| IrConst::I8(b as i8)).collect())
    }
}

// =============================================================================
// String literal helpers
// =============================================================================

impl Lowerer {
    /// Pure `&self` query matching [`Self::eval_string_literal_addr_expr`] without
    /// interning literals, used by relocation-planning predicates (`&self`).
    pub(super) fn is_string_literal_addr_expr(&self, expr: &Expr) -> bool {
        match expr {
            Expr::BinaryOp(BinOp::Add, lhs, rhs, _) => {
                self.can_eval_string_literal_with_offset(lhs, rhs)
                    || self.can_eval_string_literal_with_offset(rhs, lhs)
            }
            Expr::BinaryOp(BinOp::Sub, lhs, rhs, _) => {
                self.can_eval_string_literal_with_offset(lhs, rhs)
            }
            Expr::AddressOf(inner, _) => match inner.as_ref() {
                Expr::ArraySubscript(base, index, _) => {
                    self.can_eval_string_literal_with_offset(base, index)
                }
                Expr::StringLiteral(..)
                | Expr::WideStringLiteral(..)
                | Expr::Char16StringLiteral(..) => true,
                Expr::Identifier(..) => self.function_name_string(inner).is_some(),
                Expr::Deref(sub, _) => self.is_string_literal_addr_expr(sub),
                _ => false,
            },
            Expr::Cast(_, inner, _, _) => self.is_string_literal_addr_expr(inner),
            Expr::Conditional(cond, then, els, _) => {
                let Some(cv) = self.eval_init_condition(cond) else {
                    return false;
                };
                let taken = if cv.is_nonzero() { then } else { els };
                self.is_string_literal_addr_expr(taken)
            }
            Expr::GenericSelection(controlling, associations, _) => self
                .resolve_generic_selection_expr(controlling, associations)
                .is_some_and(|sel| self.is_string_literal_addr_expr(sel)),
            Expr::FunctionCall(callee, args, _)
                if matches!(callee.as_ref(), Expr::Identifier(name, _) if name == "__builtin_choose_expr")
                    && args.len() >= 3 =>
            {
                let Some(cv) = self.eval_const_expr(&args[0]) else {
                    return false;
                };
                let chosen = if cv.is_nonzero() { &args[1] } else { &args[2] };
                self.is_string_literal_addr_expr(chosen)
            }
            Expr::Identifier(..) => self.function_name_string(expr).is_some(),
            Expr::StringLiteral(..)
            | Expr::WideStringLiteral(..)
            | Expr::Char16StringLiteral(..) => true,
            _ => false,
        }
    }

    fn can_eval_string_literal_with_offset(&self, str_expr: &Expr, offset_expr: &Expr) -> bool {
        (self.extract_string_literal(str_expr).is_some()
            || self.function_name_string(str_expr).is_some())
            && self
                .eval_const_expr(offset_expr)
                .and_then(|v| self.const_to_i64(&v))
                .is_some()
    }

    /// Evaluate a string literal address expression for static initializers.
    /// Handles: `"hello"`, `&"hello"`, `"hello" + 2`, `"hello" - 1`,
    /// `(type*)"hello"`, `&"hello"[2]`, conditionals, `_Generic`, etc.
    pub(super) fn eval_string_literal_addr_expr(&mut self, expr: &Expr) -> Option<GlobalInit> {
        match expr {
            Expr::BinaryOp(BinOp::Add, lhs, rhs, _) => self
                .eval_string_literal_with_offset(lhs, rhs, false)
                .or_else(|| self.eval_string_literal_with_offset(rhs, lhs, false)),
            Expr::BinaryOp(BinOp::Sub, lhs, rhs, _) => {
                self.eval_string_literal_with_offset(lhs, rhs, true)
            }
            // &"string"[index] -> intern string literal + byte offset;
            // &"string" -> address of the string array object itself (same runtime
            // base address as the decayed string pointer, e.g. kernel PCI_VDEVICE's
            // `(0 ? 0 : (kernel_ulong_t)&"PARPORT_SERIAL")`).
            Expr::AddressOf(inner, _) => match inner.as_ref() {
                Expr::ArraySubscript(base, index, _) => {
                    self.eval_string_literal_with_offset(base, index, false)
                }
                Expr::StringLiteral(..)
                | Expr::WideStringLiteral(..)
                | Expr::Char16StringLiteral(..)
                | Expr::Identifier(..) => self.eval_string_literal_addr_expr(inner),
                Expr::Deref(sub, _) => self.eval_string_literal_addr_expr(sub),
                _ => None,
            },
            Expr::Cast(_, inner, _, _) => self.eval_string_literal_addr_expr(inner),
            // `c ? "a" : "b"` with a constant condition is the selected operand
            // (the kernel's `__builtin_constant_p(MOD ": ...") ? MOD ": ..." : ...`).
            Expr::Conditional(cond, then, els, _) => {
                let taken = if self.eval_init_condition(cond)?.is_nonzero() {
                    then
                } else {
                    els
                };
                self.eval_string_literal_addr_expr(taken)
            }
            Expr::GenericSelection(controlling, associations, _) => {
                let selected = self
                    .resolve_generic_selection_expr(controlling, associations)?
                    .clone();
                self.eval_string_literal_addr_expr(&selected)
            }
            Expr::FunctionCall(callee, args, _)
                if matches!(callee.as_ref(), Expr::Identifier(name, _) if name == "__builtin_choose_expr")
                    && args.len() >= 3 =>
            {
                let chosen = if self.eval_const_expr(&args[0])?.is_nonzero() {
                    &args[1]
                } else {
                    &args[2]
                };
                self.eval_string_literal_addr_expr(chosen)
            }
            Expr::Identifier(..) => {
                let (name, _) = self.function_name_string(expr)?;
                Some(GlobalInit::GlobalAddr(self.intern_string_literal(&name)))
            }
            Expr::StringLiteral(s, _) => {
                Some(GlobalInit::GlobalAddr(self.intern_string_literal(s)))
            }
            Expr::WideStringLiteral(s, _) => {
                Some(GlobalInit::GlobalAddr(self.intern_wide_string_literal(s)))
            }
            Expr::Char16StringLiteral(s, _) => {
                Some(GlobalInit::GlobalAddr(self.intern_char16_string_literal(s)))
            }
            _ => None,
        }
    }

    /// Helper: intern a string literal + offset => GlobalAddr or GlobalAddrOffset.
    fn eval_string_literal_with_offset(
        &mut self,
        str_expr: &Expr,
        offset_expr: &Expr,
        negate: bool,
    ) -> Option<GlobalInit> {
        let (s, kind) = self
            .extract_string_literal(str_expr)
            .or_else(|| self.function_name_string(str_expr))?;
        let offset_val = self.eval_const_expr(offset_expr)?;
        let offset = self.const_to_i64(&offset_val)?;
        let byte_offset = if negate { -offset } else { offset };
        let label = match kind {
            StringLitKind::Narrow => self.intern_string_literal(&s),
            StringLitKind::Wide => self.intern_wide_string_literal(&s),
            StringLitKind::Char16 => self.intern_char16_string_literal(&s),
        };
        if byte_offset == 0 {
            Some(GlobalInit::GlobalAddr(label))
        } else {
            Some(GlobalInit::GlobalAddrOffset(label, byte_offset))
        }
    }

    /// `__func__` (or `__FUNCTION__`/`__PRETTY_FUNCTION__`) inside a function,
    /// possibly through casts: the same interned string the expression path
    /// lowers it to, so `static const char *s = __func__;` compares equal to
    /// `__func__`. An address base only -- the identifier is an array object,
    /// not a string literal, so it never initializes a char array.
    fn function_name_string(&self, expr: &Expr) -> Option<(String, StringLitKind)> {
        match expr {
            Expr::Identifier(name, _)
                if crate::frontend::parser::ast::is_function_name_builtin(name) =>
            {
                let text = crate::frontend::parser::ast::function_name_builtin_text(
                    name,
                    self.func_state.as_ref().map(|fs| fs.source_name()),
                );
                Some((text.to_string(), StringLitKind::Narrow))
            }
            Expr::Cast(_, inner, _, _) => self.function_name_string(inner),
            _ => None,
        }
    }

    /// Extract a string literal from an expression, possibly through casts.
    fn extract_string_literal(&self, expr: &Expr) -> Option<(String, StringLitKind)> {
        match expr {
            Expr::StringLiteral(s, _) => Some((s.clone(), StringLitKind::Narrow)),
            Expr::WideStringLiteral(s, _) => Some((s.clone(), StringLitKind::Wide)),
            Expr::Char16StringLiteral(s, _) => Some((s.clone(), StringLitKind::Char16)),
            Expr::Cast(_, inner, _, _) => self.extract_string_literal(inner),
            _ => None,
        }
    }
}

// =============================================================================
// Compound literal globals
// =============================================================================

impl Lowerer {
    /// Materialize (or reuse) an anonymous static/TLS global for a C23
    /// `(static [thread_local] [const] T){...}` compound literal.
    pub(super) fn ensure_static_compound_literal_global(
        &mut self,
        cl_expr: &Expr,
        type_spec: &TypeSpecifier,
        init: &Initializer,
        storage: &crate::frontend::parser::ast::CompoundLiteralStorage,
    ) -> String {
        let key = cl_expr as *const Expr as usize;
        if let Some(existing) = self.materialized_compound_literals.get(&key) {
            return existing.clone();
        }
        let init_res = self.create_compound_literal_global_with_storage(type_spec, init, storage);
        match init_res {
            GlobalInit::GlobalAddr(label) => {
                self.materialized_compound_literals
                    .insert(key, label.clone());
                label
            }
            _ => String::new(),
        }
    }

    /// Create an anonymous global for a compound literal at file scope.
    pub(super) fn create_compound_literal_global(
        &mut self,
        type_spec: &TypeSpecifier,
        init: &Initializer,
    ) -> GlobalInit {
        let default_st = crate::frontend::parser::ast::CompoundLiteralStorage::default();
        self.create_compound_literal_global_with_storage(type_spec, init, &default_st)
    }

    pub(super) fn create_compound_literal_global_with_storage(
        &mut self,
        type_spec: &TypeSpecifier,
        init: &Initializer,
        storage: &crate::frontend::parser::ast::CompoundLiteralStorage,
    ) -> GlobalInit {
        let label = if storage.is_thread_local {
            format!("__compound_lit_tls.{}", self.next_anon_struct)
        } else {
            format!(".Lcompound_lit_{}", self.next_anon_struct)
        };
        self.next_anon_struct += 1;

        let is_array = matches!(type_spec, TypeSpecifier::Array(_, _));
        let (elem_size, base_ty, computed_alloc_size) =
            if let TypeSpecifier::Array(elem_ts, bound) = type_spec {
                let elem_ir_ty = self.type_spec_to_ir(elem_ts);
                let e_size = self.sizeof_type(elem_ts);
                // A sized array literal has its declared size whatever the
                // initializer covers: `(int[4]){ 1 }` is 16 bytes.
                let size = if bound.is_some() {
                    self.sizeof_type(type_spec)
                } else {
                    let elem_ct = self.type_spec_to_ctype(elem_ts);
                    match (self.unsized_array_len(init, &elem_ct), init) {
                        (Some(n), _) => e_size.saturating_mul(n),
                        (None, Initializer::List(items)) => {
                            e_size * self.compute_init_list_array_size(items)
                        }
                        (None, Initializer::Expr(_)) => e_size,
                    }
                };
                (e_size, elem_ir_ty, size)
            } else {
                let ty = self.type_spec_to_ir(type_spec);
                let size = self.sizeof_type(type_spec);
                (0, ty, size)
            };

        let ctype = self.type_spec_to_ctype(type_spec);
        let struct_layout = self.get_struct_layout_for_type(type_spec);
        // For arrays, struct_layout is for the element type, not the whole array,
        // so always use computed_alloc_size which accounts for the element count.
        let alloc_size = if is_array {
            computed_alloc_size
        } else {
            struct_layout
                .as_ref()
                .map_or(computed_alloc_size, |l| l.size)
        };
        let align = struct_layout
            .as_ref()
            .map_or(base_ty.align(), |l| l.align.max(base_ty.align()));

        let global_init = self.lower_global_init(
            init,
            type_spec,
            base_ty,
            is_array,
            elem_size,
            alloc_size,
            &struct_layout,
            &[],
            Some(&ctype),
        );

        let global_ty = if matches!(&global_init, GlobalInit::Array(vals) if !vals.is_empty() && matches!(vals[0], IrConst::I8(_)))
            || (struct_layout.is_some() && matches!(global_init, GlobalInit::Array(_)))
        {
            IrType::I8
        } else {
            base_ty
        };

        self.emitted_global_names.insert(label.clone());
        self.module.globals.push(IrGlobal {
            name: label.clone(),
            ty: global_ty,
            size: alloc_size,
            align,
            init: global_init,
            is_static: true,
            is_extern: false,
            is_common: false,
            section: None,
            is_weak: false,
            visibility: None,
            has_explicit_align: false,
            is_const: (storage.is_const || storage.is_constexpr) && !storage.is_thread_local,
            is_used: false,
            is_retain: false,
            is_nocommon: false,
            is_thread_local: storage.is_thread_local,
        });

        GlobalInit::GlobalAddr(label)
    }
}

// =============================================================================
// Struct initializer helpers
// =============================================================================

impl Lowerer {
    fn lower_struct_global_init(
        &mut self,
        items: &[InitializerItem],
        layout: &StructLayout,
    ) -> GlobalInit {
        if self.init_needs_relocations(items, layout) {
            return self.lower_struct_global_init_compound(items, layout);
        }

        let total_size = layout.size + self.compute_fam_extra_size(items, layout);
        let mut bytes = vec![0u8; total_size];
        self.fill_struct_global_bytes(items, layout, &mut bytes, 0);
        let values: Vec<IrConst> = bytes.iter().map(|&b| IrConst::I8(b as i8)).collect();
        GlobalInit::Array(values)
    }

    /// Compute extra bytes needed for a flexible array member (FAM).
    pub(super) fn compute_fam_extra_size(
        &self,
        items: &[InitializerItem],
        layout: &StructLayout,
    ) -> usize {
        let last_field = match layout.fields.last() {
            Some(f) => f,
            None => return 0,
        };
        let (elem_ty, last_field_idx) = match &last_field.ty {
            CType::Array(elem_ty, None) => (elem_ty, layout.fields.len() - 1),
            _ => return 0,
        };
        let elem_size = self.resolve_ctype_size(elem_ty);
        let mut current_field_idx = 0usize;
        for (item_idx, item) in items.iter().enumerate() {
            let field_idx = self.resolve_struct_init_field_idx(item, layout, current_field_idx);
            if field_idx == last_field_idx {
                if let Some(geo) = self.fam_scalar_geometry(elem_ty, &item.init) {
                    return geo.total * geo.elem_size();
                }
                let num_elems = match &item.init {
                    Initializer::List(sub_items) => sub_items.len(),
                    Initializer::Expr(Expr::StringLiteral(s, _)) => {
                        if matches!(
                            elem_ty.as_ref(),
                            CType::Char | CType::SChar | CType::UChar | CType::PlainUChar
                        ) {
                            s.len() + 1
                        } else {
                            items.len() - item_idx
                        }
                    }
                    Initializer::Expr(_) => items.len() - item_idx,
                };
                return num_elems * elem_size;
            }
            if field_idx < layout.fields.len() {
                current_field_idx = field_idx + 1;
            }
        }
        0
    }

    /// Does this initializer need a relocation-aware image (bytes plus
    /// pointer-sized relocations) rather than a plain byte image?
    ///
    /// Two independent questions, and the image must satisfy both:
    ///   * can any member, at any nesting depth, hold a relocation?  A pointer
    ///     or function member always can; a member of integer type can when it
    ///     is exactly pointer-sized (C11 6.6p9 -- the kernel's
    ///     `x86_cpu_id.driver_data` is `unsigned long`);
    ///   * does the initializer actually store an address anywhere?
    ///
    /// `StructLayout::has_pointer_fields` answers only the pointer half of the
    /// first question, and it is the right predicate *there* (it decides ABI
    /// class and copy shape).  Here the question is what the image can express,
    /// which is why a struct with no pointer member can still need relocations.
    /// Being conservative is correct and cheap: an initializer that stores no
    /// address produces byte-identical output on either path.
    pub(super) fn init_needs_relocations(
        &self,
        items: &[InitializerItem],
        layout: &StructLayout,
    ) -> bool {
        if layout.has_pointer_fields(&*self.types.borrow_struct_layouts()) {
            return true;
        }
        self.layout_can_hold_addr_constant(layout)
            && items
                .iter()
                .any(|item| self.init_has_addr_exprs(&item.init))
    }

    /// The struct-layout equivalent of `ctype_can_hold_addr_constant`: does any
    /// member of `layout`, at any nesting depth, have a type that can hold an
    /// address constant?
    pub(super) fn layout_can_hold_addr_constant(&self, layout: &StructLayout) -> bool {
        layout
            .fields
            .iter()
            .any(|field| self.ctype_can_hold_addr_constant(&field.ty))
    }

    /// Can a member of this type hold a relocation, i.e. an address constant?
    ///
    /// A pointer or function member obviously can.  So can a member of integer
    /// type that is exactly pointer-sized: C11 6.6p9 admits an address constant
    /// cast to an integer type, and the kernel keeps its CPU tables that way
    /// (`struct x86_cpu_id`'s `unsigned long driver_data` initialized with
    /// `(unsigned long)&model`, arch/x86/include/asm/cpu_device_id.h).  A
    /// narrower integer member cannot -- GCC and Clang both reject the
    /// truncating cast as a non-constant initializer -- so it is not counted.
    ///
    /// Recursive through aggregates: the member that receives the address may
    /// sit inside a nested struct, union or array of an array element.
    pub(super) fn ctype_can_hold_addr_constant(&self, ty: &CType) -> bool {
        match ty {
            CType::Pointer(_, _) | CType::Function(_) => true,
            CType::Struct(_) | CType::Union(_) => self
                .get_struct_layout_for_ctype(ty)
                .is_some_and(|layout| self.layout_can_hold_addr_constant(&layout)),
            CType::Array(elem, _) => self.ctype_can_hold_addr_constant(elem),
            other => {
                other.is_integer()
                    && self.resolve_ctype_size(other) == crate::common::types::target_ptr_size()
            }
        }
    }

    /// Check if a struct initializer contains fields needing address relocations.
    pub(super) fn struct_init_has_addr_fields(
        &self,
        items: &[InitializerItem],
        layout: &StructLayout,
    ) -> bool {
        let mut current_field_idx = 0usize;
        let mut item_idx = 0usize;

        while item_idx < items.len() {
            let item = &items[item_idx];
            let field_idx = self.resolve_struct_init_field_idx(item, layout, current_field_idx);

            if field_idx >= layout.fields.len() {
                // Item is beyond all fields. Still check if it has address expressions
                // since brace-elided counting may have caused us to skip past the layout.
                if self.init_has_addr_exprs(&item.init) {
                    return true;
                }
                item_idx += 1;
                continue;
            }

            let field_ty = &layout.fields[field_idx].ty;

            // Arrays of scalars: consume the member's initializer exactly as
            // the writers do (`member_array_init`) and ask the plan.
            let desig = h::first_field_designator(item);
            if desig.is_some() || item.designators.is_empty() {
                if let Some(geo) = self.scalar_array_geometry(field_ty) {
                    let rel: &[Designator] = if desig.is_some() {
                        &item.designators[1..]
                    } else {
                        &[]
                    };
                    let (member, consumed) =
                        self.member_array_init(rel, &items[item_idx..], field_ty, &geo);
                    let plan = self.plan_member_array(&member, &geo);
                    if self.plan_needs_relocations(&plan, &geo) {
                        return true;
                    }
                    item_idx += consumed;
                    current_field_idx = field_idx + 1;
                    continue;
                }
            }

            if h::has_nested_field_designator(item) {
                if self.nested_designator_has_addr_fields(item, field_ty) {
                    return true;
                }
                current_field_idx = field_idx + 1;
                item_idx += 1;
                continue;
            }

            if self.field_init_has_addr_refs(item, field_ty, items, item_idx) {
                return true;
            }

            // For brace-elided sub-struct fields, advance by the number of
            // scalar leaf fields, checking each consumed item for address exprs.
            if matches!(field_ty, CType::Struct(_) | CType::Union(_))
                && matches!(&item.init, Initializer::Expr(_))
                && item.designators.is_empty()
            {
                let scalars_needed =
                    h::count_flat_init_scalars(field_ty, &*self.types.borrow_struct_layouts());
                if scalars_needed > 1 {
                    // Check remaining consumed items for address expressions
                    for offset in 1..scalars_needed {
                        let idx = item_idx + offset;
                        if idx >= items.len() {
                            break;
                        }
                        if !items[idx].designators.is_empty() {
                            break;
                        }
                        if self.init_has_addr_exprs(&items[idx].init) {
                            return true;
                        }
                    }
                    item_idx += scalars_needed;
                    current_field_idx = field_idx + 1;
                    continue;
                }
            }

            // Also for array fields with flat init, advance by the total scalars count
            if let CType::Array(elem_ty, Some(arr_size)) = field_ty {
                if matches!(&item.init, Initializer::Expr(_)) && item.designators.is_empty() {
                    let is_string_literal =
                        matches!(&item.init, Initializer::Expr(Expr::StringLiteral(..)));
                    let is_char_array = matches!(
                        elem_ty.as_ref(),
                        CType::Char | CType::SChar | CType::UChar | CType::PlainUChar
                    );
                    if !(is_string_literal && is_char_array) {
                        let scalars_per_elem = h::count_flat_init_scalars(
                            elem_ty,
                            &*self.types.borrow_struct_layouts(),
                        );
                        let total_scalars = arr_size * scalars_per_elem;
                        // Check all consumed items for address expressions
                        for offset in 0..total_scalars {
                            let idx = item_idx + offset;
                            if idx >= items.len() {
                                break;
                            }
                            if !items[idx].designators.is_empty() && offset > 0 {
                                break;
                            }
                            if self.init_has_addr_exprs(&items[idx].init) {
                                return true;
                            }
                        }
                        item_idx += total_scalars.min(items.len() - item_idx);
                        current_field_idx = field_idx + 1;
                        continue;
                    }
                }
            }

            current_field_idx = field_idx + 1;
            item_idx += 1;
        }
        false
    }

    /// Check if a single field's initializer contains address references.
    fn field_init_has_addr_refs(
        &self,
        item: &InitializerItem,
        field_ty: &CType,
        items: &[InitializerItem],
        item_idx: usize,
    ) -> bool {
        match &item.init {
            Initializer::Expr(expr) => {
                if Self::expr_contains_label_addr(expr) {
                    return true;
                }
                if h::expr_contains_string_literal(expr)
                    && (h::type_has_pointer_elements(
                        field_ty,
                        &*self.types.borrow_struct_layouts(),
                    ) || (self.ctype_can_hold_addr_constant(field_ty)
                        && self.eval_const_expr(expr).is_none()
                        && self.is_string_literal_addr_expr(expr)))
                {
                    return true;
                }
                if self.eval_const_expr(expr).is_none()
                    && self.eval_global_addr_expr(expr).is_some()
                {
                    return true;
                }
                if let Expr::AddressOf(inner, _) = expr {
                    if matches!(inner.as_ref(), Expr::CompoundLiteral(..)) {
                        return true;
                    }
                }
                // Bare compound literal initializing a pointer field (array-to-pointer decay)
                if let Expr::CompoundLiteral(..) = expr {
                    if matches!(field_ty, CType::Pointer(_, _)) {
                        return true;
                    }
                }
                // Check flat array elements following this item
                if let CType::Array(elem_ty, Some(arr_size)) = field_ty {
                    if h::type_has_pointer_elements(elem_ty, &*self.types.borrow_struct_layouts()) {
                        for i in 1..*arr_size {
                            let next = item_idx + i;
                            if next >= items.len() {
                                break;
                            }
                            if !items[next].designators.is_empty() {
                                break;
                            }
                            if self.init_has_addr_exprs(&items[next].init) {
                                return true;
                            }
                        }
                    }
                }
                false
            }
            Initializer::List(nested_items) => {
                if h::type_has_pointer_elements(field_ty, &*self.types.borrow_struct_layouts())
                    && self.init_has_addr_exprs(&item.init)
                {
                    return true;
                }
                if let Some(nested_layout) = self.get_struct_layout_for_ctype(field_ty) {
                    if self.struct_init_has_addr_fields(nested_items, &nested_layout) {
                        return true;
                    }
                }
                false
            }
        }
    }

    /// Check multi-level designated initializer for address fields.
    fn nested_designator_has_addr_fields(&self, item: &InitializerItem, outer_ty: &CType) -> bool {
        let drill = match self.drill_designators(&item.designators[1..], outer_ty) {
            Some(d) => d,
            None => return false,
        };
        let current_ty = drill.target_ty;

        if matches!(&current_ty, CType::Pointer(_, _) | CType::Function(_)) {
            return self.init_has_addr_exprs(&item.init);
        }

        if let Some(target_layout) = self.get_struct_layout_for_ctype(&current_ty) {
            if target_layout.has_pointer_fields(&*self.types.borrow_struct_layouts())
                && self.init_has_addr_exprs(&item.init)
            {
                return true;
            }
            if let Initializer::List(nested_items) = &item.init {
                if self.struct_init_has_addr_fields(nested_items, &target_layout) {
                    return true;
                }
            }
        }

        if let CType::Array(elem_ty, _) = &current_ty {
            if h::type_has_pointer_elements(elem_ty, &*self.types.borrow_struct_layouts()) {
                return self.init_has_addr_exprs(&item.init);
            }
        }

        false
    }

    /// Check if an initializer contains address expressions (recursive).
    fn init_has_addr_exprs(&self, init: &Initializer) -> bool {
        match init {
            Initializer::Expr(expr) => {
                if Self::expr_contains_label_addr(expr) {
                    return true;
                }
                if h::expr_contains_string_literal(expr) {
                    return true;
                }
                if let Expr::AddressOf(inner, _) = expr {
                    if matches!(inner.as_ref(), Expr::CompoundLiteral(..)) {
                        return true;
                    }
                }
                if let Expr::CompoundLiteral(_, cl_init, _, _) = expr {
                    return self.init_has_addr_exprs(cl_init);
                }
                self.eval_const_expr(expr).is_none() && self.eval_global_addr_expr(expr).is_some()
            }
            Initializer::List(items) => items
                .iter()
                .any(|item| self.init_has_addr_exprs(&item.init)),
        }
    }
}

// =============================================================================
// Array flattening (multi-dimensional designated initializers)
// =============================================================================

impl Lowerer {
    /// Write values from `src` into `dest` starting at `start_idx`, overwriting
    /// existing entries or extending the vector as needed.
    fn overwrite_or_extend_values(
        dest: &mut Vec<IrConst>,
        start_idx: usize,
        src: Vec<IrConst>,
        zero: &dyn Fn() -> IrConst,
    ) {
        for (i, v) in src.into_iter().enumerate() {
            let pos = start_idx + i;
            if pos < dest.len() {
                dest[pos] = v;
            } else {
                while dest.len() < pos {
                    dest.push(zero());
                }
                dest.push(v);
            }
        }
    }

    /// Inline a string literal into a values array for a sub-array element.
    fn inline_string_to_values(
        &self,
        s: &str,
        sub_elem_count: usize,
        base_ty: IrType,
        values: &mut Vec<IrConst>,
    ) {
        let start_len = values.len();
        for c in s.chars() {
            values.push(IrConst::I64(c as u8 as i64));
        }
        if values.len() < start_len + sub_elem_count {
            values.push(IrConst::I64(0));
        }
        while values.len() < start_len + sub_elem_count {
            values.push(self.zero_const(base_ty));
        }
        if values.len() > start_len + sub_elem_count {
            values.truncate(start_len + sub_elem_count);
        }
    }

    fn flatten_global_array_init_bool(
        &self,
        items: &[InitializerItem],
        array_dim_strides: &[usize],
        base_ty: IrType,
        values: &mut Vec<IrConst>,
        elem_ct: &CType,
    ) {
        let base_type_size = base_ty.size().max(1);
        if array_dim_strides.len() <= 1 {
            self.flatten_1d_array(items, base_ty, values, elem_ct);
            return;
        }

        let sub_elem_count = if array_dim_strides[0] > 0 && base_type_size > 0 {
            array_dim_strides[0] / base_type_size
        } else {
            1
        };
        let start_len = values.len();
        let mut current_outer_idx = 0usize;

        for item in items {
            let index_designators: Vec<usize> = item
                .designators
                .iter()
                .filter_map(|d| {
                    if let Designator::Index(idx_expr) = d {
                        self.eval_const_expr(idx_expr).and_then(|c| c.to_usize())
                    } else {
                        None
                    }
                })
                .collect();

            if !index_designators.is_empty() {
                self.flatten_designated_item(
                    item,
                    &index_designators,
                    array_dim_strides,
                    base_ty,
                    base_type_size,
                    sub_elem_count,
                    values,
                    elem_ct,
                );
                current_outer_idx = index_designators[0] + 1;
                continue;
            }

            // Sequential processing (no designator)
            self.flatten_sequential_item(
                item,
                array_dim_strides,
                base_ty,
                sub_elem_count,
                start_len,
                &mut current_outer_idx,
                values,
                elem_ct,
            );
        }
    }

    /// Flatten a 1D array with designated initializer support.
    fn flatten_1d_array(
        &self,
        items: &[InitializerItem],
        base_ty: IrType,
        values: &mut Vec<IrConst>,
        elem_ct: &CType,
    ) {
        let mut current_idx = 0usize;
        for item in items {
            if let Some(Designator::Index(idx_expr)) = item.designators.first() {
                if let Some(idx) = self.eval_const_expr(idx_expr).and_then(|c| c.to_usize()) {
                    current_idx = idx;
                }
            }
            // Pad up to current_idx if needed (forward jump)
            while values.len() < current_idx {
                values.push(self.zero_const(base_ty));
            }
            if current_idx < values.len() {
                // Backward jump or overwrite
                let mut tmp = Vec::new();
                self.flatten_global_init_item(&item.init, base_ty, &mut tmp, elem_ct);
                let n = tmp.len();
                let zero_fn = || self.zero_const(base_ty);
                Self::overwrite_or_extend_values(values, current_idx, tmp, &zero_fn);
                current_idx += n.max(1);
            } else {
                // Forward: append at end
                self.flatten_global_init_item(&item.init, base_ty, values, elem_ct);
                current_idx = values.len();
            }
        }
    }

    /// Flatten a designated item in a multi-dimensional array.
    fn flatten_designated_item(
        &self,
        item: &InitializerItem,
        index_designators: &[usize],
        array_dim_strides: &[usize],
        base_ty: IrType,
        base_type_size: usize,
        sub_elem_count: usize,
        values: &mut Vec<IrConst>,
        elem_ct: &CType,
    ) {
        let flat_idx = self.compute_flat_index_from_designators(
            index_designators,
            array_dim_strides,
            base_type_size,
        );
        // Pad up to flat_idx if needed
        while values.len() <= flat_idx {
            values.push(self.zero_const(base_ty));
        }

        let zero_fn = || self.zero_const(base_ty);
        match &item.init {
            Initializer::List(sub_items) => {
                let remaining_dims = array_dim_strides
                    .len()
                    .saturating_sub(index_designators.len());
                let sub_strides = &array_dim_strides[array_dim_strides.len() - remaining_dims..];
                let mut tmp = Vec::new();
                if sub_strides.is_empty() || remaining_dims == 0 {
                    self.flatten_global_init_item(&item.init, base_ty, &mut tmp, elem_ct);
                } else {
                    self.flatten_global_array_init_bool(
                        sub_items,
                        sub_strides,
                        base_ty,
                        &mut tmp,
                        elem_ct,
                    );
                }
                Self::overwrite_or_extend_values(values, flat_idx, tmp, &zero_fn);
            }
            Initializer::Expr(expr) => {
                if let Expr::StringLiteral(s, _) = expr {
                    // For designated string literal in a multi-dim char array:
                    // e.g., char arr[2][5] = { [0] = "ABCD" }
                    // index_designators = [0], array_dim_strides = [5, 1]
                    // We need to fill 5 chars (the inner array size).
                    // Use the stride at the designator level (strides[0] = 5), not after it.
                    let string_sub_count = if !index_designators.is_empty()
                        && index_designators.len() < array_dim_strides.len()
                        && base_type_size > 0
                    {
                        // Stride at the last designator level gives the size of the target element
                        array_dim_strides[index_designators.len() - 1] / base_type_size
                    } else if index_designators.len() < array_dim_strides.len() {
                        sub_elem_count
                    } else {
                        1
                    };
                    let mut tmp = Vec::new();
                    self.inline_string_to_values(s, string_sub_count, base_ty, &mut tmp);
                    Self::overwrite_or_extend_values(values, flat_idx, tmp, &zero_fn);
                } else if let Some(val) = self.eval_const_expr(expr) {
                    values[flat_idx] = self.coerce_init_const(val, expr, base_ty, Some(elem_ct));
                }
            }
        }
    }

    /// Flatten a sequential (non-designated) item in a multi-dimensional array.
    fn flatten_sequential_item(
        &self,
        item: &InitializerItem,
        array_dim_strides: &[usize],
        base_ty: IrType,
        sub_elem_count: usize,
        start_len: usize,
        current_outer_idx: &mut usize,
        values: &mut Vec<IrConst>,
        elem_ct: &CType,
    ) {
        match &item.init {
            Initializer::List(sub_items) => {
                let target_start = start_len + *current_outer_idx * sub_elem_count;
                while values.len() < target_start {
                    values.push(self.zero_const(base_ty));
                }
                // Check for braced string literal initializing a char sub-array
                if sub_items.len() == 1 {
                    if let Initializer::Expr(Expr::StringLiteral(s, _)) = &sub_items[0].init {
                        self.inline_string_to_values(s, sub_elem_count, base_ty, values);
                        *current_outer_idx += 1;
                        return;
                    }
                }
                let mut sub_values = Vec::with_capacity(sub_elem_count);
                self.flatten_global_array_init_bool(
                    sub_items,
                    &array_dim_strides[1..],
                    base_ty,
                    &mut sub_values,
                    elem_ct,
                );
                while sub_values.len() < sub_elem_count {
                    sub_values.push(self.zero_const(base_ty));
                }
                values.extend(sub_values.into_iter().take(sub_elem_count));
                *current_outer_idx += 1;
            }
            Initializer::Expr(expr) => {
                if let Expr::StringLiteral(s, _) = expr {
                    let target_start = start_len + *current_outer_idx * sub_elem_count;
                    while values.len() < target_start {
                        values.push(self.zero_const(base_ty));
                    }
                    self.inline_string_to_values(s, sub_elem_count, base_ty, values);
                    *current_outer_idx += 1;
                } else if let Some(val) = self.eval_const_expr(expr) {
                    values.push(self.coerce_init_const(val, expr, base_ty, Some(elem_ct)));
                    let relative_pos = values.len() - start_len;
                    if sub_elem_count > 0 {
                        *current_outer_idx = relative_pos.div_ceil(sub_elem_count);
                    }
                } else {
                    values.push(self.zero_const(base_ty));
                    let relative_pos = values.len() - start_len;
                    if sub_elem_count > 0 {
                        *current_outer_idx = relative_pos.div_ceil(sub_elem_count);
                    }
                }
            }
        }
    }

    /// Compute a flat index from multi-dimensional designator indices.
    fn compute_flat_index_from_designators(
        &self,
        indices: &[usize],
        array_dim_strides: &[usize],
        base_type_size: usize,
    ) -> usize {
        let mut flat_idx = 0usize;
        for (i, &idx) in indices.iter().enumerate() {
            let elems_per_entry = if i < array_dim_strides.len() && base_type_size > 0 {
                array_dim_strides[i] / base_type_size
            } else {
                1
            };
            flat_idx += idx * elems_per_entry;
        }
        flat_idx
    }
}

// =============================================================================
// Misc helpers
// =============================================================================

impl Lowerer {
    /// Promote an IrConst value to LongDouble.
    fn promote_to_long_double(val: IrConst) -> IrConst {
        match val {
            IrConst::F64(v) => IrConst::long_double(v),
            IrConst::F32(v) => IrConst::long_double(v as f64),
            IrConst::I64(v) => IrConst::long_double_from_i64(v),
            IrConst::I32(v) => IrConst::long_double_from_i64(v as i64),
            IrConst::I16(v) => IrConst::long_double_from_i64(v as i64),
            IrConst::I8(v) => IrConst::long_double_from_i64(v as i64),
            other => other,
        }
    }

    /// Conditionally promote a value to long double.
    pub(super) fn maybe_promote_long_double(val: IrConst, is_long_double: bool) -> IrConst {
        if is_long_double {
            Self::promote_to_long_double(val)
        } else {
            val
        }
    }

    /// Get the appropriate zero constant for a type, considering long double.
    pub(super) fn typed_zero_const(&self, base_ty: IrType, is_long_double: bool) -> IrConst {
        if is_long_double {
            IrConst::long_double(0.0)
        } else {
            self.zero_const(base_ty)
        }
    }

    /// Collect a single compound initializer element, handling nested lists.
    pub(super) fn collect_compound_init_element(
        &mut self,
        init: &Initializer,
        elements: &mut Vec<GlobalInit>,
        elem_size: usize,
    ) {
        match init {
            Initializer::Expr(expr) => {
                // Resolve _Generic selections to their selected expression
                // before any other processing.
                let expr = if let Expr::GenericSelection(controlling, associations, _) = expr {
                    if let Some(selected) =
                        self.resolve_generic_selection_expr(controlling, associations)
                    {
                        selected
                    } else {
                        expr
                    }
                } else {
                    expr
                };
                if let Expr::StringLiteral(s, _) = expr {
                    let label = self.intern_string_literal(s);
                    elements.push(GlobalInit::GlobalAddr(label));
                } else if let Expr::LabelAddr(label_name, _) = Self::strip_casts(expr) {
                    let scoped_label = self.get_or_create_user_label(label_name);
                    if let Some(ref mut fs) = self.func_state {
                        fs.global_init_label_blocks.push(scoped_label);
                    }
                    elements.push(GlobalInit::GlobalAddr(scoped_label.as_label()));
                // &(compound_literal) or cast-wrapped variant -> materialize and take address
                } else if let Some(addr) = self.try_address_of_compound_literal(expr) {
                    elements.push(addr);
                } else if let Some(val) = self.eval_const_expr(expr) {
                    let val = match val {
                        IrConst::I32(v) if elem_size == 8 => IrConst::I64(v as i64),
                        IrConst::I16(v) if elem_size >= 4 => {
                            if elem_size == 8 {
                                IrConst::I64(v as i64)
                            } else {
                                IrConst::I32(v as i32)
                            }
                        }
                        other => other,
                    };
                    elements.push(GlobalInit::Scalar(val));
                } else if let Some(label_diff) = self.eval_label_diff_expr(expr, elem_size) {
                    elements.push(label_diff);
                } else if let Some(addr) = self.eval_string_literal_addr_expr(expr) {
                    elements.push(addr);
                } else {
                    // Pre-materialize compound literals in arithmetic before resolving
                    self.materialize_compound_literals_in_expr(expr);
                    if let Some(addr) = self.eval_global_addr_expr(expr) {
                        elements.push(addr);
                    } else {
                        self.diagnose_non_constant(expr);
                        elements.push(GlobalInit::Zero);
                    }
                }
            }
            Initializer::List(sub_items) => {
                if sub_items.is_empty() {
                    elements.push(GlobalInit::Zero);
                } else {
                    for sub in sub_items {
                        self.collect_compound_init_element(&sub.init, elements, elem_size);
                    }
                }
            }
        }
    }

    /// Try to evaluate a label difference: `&&lab1 - &&lab2`.
    fn eval_label_diff_expr(&mut self, expr: &Expr, byte_size: usize) -> Option<GlobalInit> {
        self.func_state.as_ref()?;
        if let Expr::BinaryOp(BinOp::Sub, lhs, rhs, _) = expr {
            let lhs_inner = Self::strip_casts(lhs);
            let rhs_inner = Self::strip_casts(rhs);
            if let (Expr::LabelAddr(lab1, _), Expr::LabelAddr(lab2, _)) = (lhs_inner, rhs_inner) {
                let scoped1 = self.get_or_create_user_label(lab1);
                let scoped2 = self.get_or_create_user_label(lab2);
                let func_name = self.func().name.clone();
                let resolved1 = self.resolve_local_label(lab1);
                let resolved2 = self.resolve_local_label(lab2);
                let alias1 = format!(".LLDIFF.{}.{}", func_name, resolved1);
                let alias2 = format!(".LLDIFF.{}.{}", func_name, resolved2);
                if let Some(ref mut fs) = self.func_state {
                    fs.global_init_label_blocks.push(scoped1);
                    fs.global_init_label_blocks.push(scoped2);
                    for (resolved, alias) in [
                        (resolved1.clone(), alias1.clone()),
                        (resolved2.clone(), alias2.clone()),
                    ] {
                        let aliases = fs.pending_label_aliases.entry(resolved).or_default();
                        if !aliases.contains(&alias) {
                            aliases.push(alias);
                        }
                    }
                }
                // Basic-block labels may be renumbered by CFG cleanup. Named
                // aliases are emitted at the label statements and remain
                // stable, so static label-difference data never goes stale.
                return Some(GlobalInit::GlobalLabelDiff(alias1, alias2, byte_size));
            }
        }
        None
    }

    /// Strip cast expressions to find the underlying expression.
    pub(super) fn strip_casts(expr: &Expr) -> &Expr {
        match expr {
            Expr::Cast(_, inner, _, _) => Self::strip_casts(inner),
            _ => expr,
        }
    }

    /// Check if an expression tree contains label addresses.
    pub(super) fn expr_contains_label_addr(expr: &Expr) -> bool {
        match expr {
            Expr::LabelAddr(_, _) => true,
            Expr::BinaryOp(_, lhs, rhs, _) => {
                Self::expr_contains_label_addr(lhs) || Self::expr_contains_label_addr(rhs)
            }
            Expr::Cast(_, inner, _, _) => Self::expr_contains_label_addr(inner),
            _ => false,
        }
    }

    /// Flatten a single initializer item, recursing into nested lists; every
    /// scalar leaf converts by value to `elem_ct`.
    fn flatten_global_init_item(
        &self,
        init: &Initializer,
        base_ty: IrType,
        values: &mut Vec<IrConst>,
        elem_ct: &CType,
    ) {
        match init {
            Initializer::Expr(expr) => {
                if let Expr::StringLiteral(s, _) = expr {
                    for c in s.chars() {
                        values.push(IrConst::I64(c as u8 as i64));
                    }
                    values.push(IrConst::I64(0));
                } else if let Some(val) = self.eval_const_expr(expr) {
                    values.push(self.coerce_init_const(val, expr, base_ty, Some(elem_ct)));
                } else {
                    values.push(self.zero_const(base_ty));
                }
            }
            Initializer::List(items) => {
                for item in items {
                    self.flatten_global_init_item(&item.init, base_ty, values, elem_ct);
                }
            }
        }
    }
}
