//! Constant expression evaluation for semantic analysis.
//!
//! This module provides compile-time constant evaluation using only sema-available
//! state (TypeContext, SymbolTable, ExprTypeChecker). It returns `IrConst` values
//! matching the lowerer's richer result type, enabling:
//! - Float literal evaluation (F32, F64, LongDouble)
//! - Proper cast chain evaluation with bit-width tracking
//! - Binary operations with type-aware signedness semantics
//! - sizeof/alignof via sema's type resolution
//!
//! Shared pure-evaluation logic (literal eval, builtin folding, sub-int promotion,
//! binary operation arithmetic) lives in `common::const_eval` and `common::const_arith`,
//! called by both this module and `ir::lowering::const_eval`.
//!
//! The lowerer's const_eval.rs handles additional lowering-specific cases:
//! - Global address expressions (&x, func, arr)
//! - func_state const local values
//! - Pointer arithmetic on global addresses
//!   These remain in the lowerer since they require IR-level state.

use super::analysis::FunctionInfo;
use super::type_context::TypeContext;
use crate::common::const_arith;
use crate::common::const_eval as shared_const_eval;
use crate::common::fx_hash::FxHashMap;
use crate::common::symbol_table::{FoldedValue, SymbolTable};
use crate::common::types::{CType, ExtVectorLayout};
use crate::frontend::parser::ast::{
    BinOp, DerivedDeclarator, Expr, ExprId, SizeofArg, StructFieldDecl, TypeSpecifier, UnaryOp,
};
use crate::ir::reexports::IrConst;

/// Map from AST expression node identity to its pre-computed compile-time constant.
///
/// Keyed by [`ExprId`], the same type-safe identity key used by `ExprTypeMap`.
/// During sema's AST walk, expressions that can be evaluated at compile time
/// have their `IrConst` values stored here. The lowerer consults this map as
/// an O(1) fast path before falling back to its own `eval_const_expr`.
pub type ConstMap = FxHashMap<ExprId, IrConst>;

/// Constant expression evaluator using only sema-available state.
///
/// This evaluator is created fresh for each expression evaluation, borrowing
/// the sema state it needs. It is stateless and does not modify anything.
///
/// When `const_values` and `expr_types` caches are provided, previously-computed
/// results are returned in O(1) instead of re-traversing the AST. This prevents
/// exponential blowup on deeply nested expressions.
pub struct SemaConstEval<'a> {
    /// Type context for typedef, enum, and struct layout resolution.
    pub types: &'a TypeContext,
    /// Symbol table for variable type lookup.
    pub symbols: &'a SymbolTable,
    /// Function signatures for return type resolution in sizeof(expr).
    pub functions: &'a FxHashMap<String, FunctionInfo>,
    /// Pre-computed constant values from bottom-up sema walk (memoization cache).
    pub const_values: Option<&'a FxHashMap<ExprId, IrConst>>,
    /// Pre-computed expression types from bottom-up sema walk.
    pub expr_types: Option<&'a FxHashMap<ExprId, CType>>,
    /// The TU's builtin policy (the CLI's `-fno-builtin` family): bare
    /// library spellings must not fold when the policy withdraws them, in
    /// constant expressions exactly as in static initializers.
    pub builtin_policy: crate::common::pass_policy::BuiltinFlags,
    /// Set while the initializer of an object with static storage is
    /// evaluated (GCC's `folding_initializer`; the lowerer's flag of the same
    /// name): `__builtin_constant_p` must then answer now, 0 unless proven,
    /// because nothing later can prove it (GCC 16.2 accepts `static int b =
    /// __builtin_constant_p(n++);` with `b == 0`, `n` untouched).  Elsewhere
    /// an unproven answer stays deferred for inlining and propagation.
    pub folding_initializer: bool,
    /// The TU is C90/C94 (`-std=c89`/`gnu89`), whose integer constant
    /// expressions admit an overflowing signed left shift; passed on to the
    /// type checker's ICE test (see `ice.rs`).
    pub pre_c99: bool,
    /// Only C23 named constants (6.6p7) name values: enumerators and
    /// `constexpr` objects. A plain `const` object's folded value -- GCC's
    /// static-initializer extension -- is no constant here (`const int k = 1;
    /// _Static_assert(k, "");` is "not constant": GCC 16.2). Set for static
    /// assertions; the bottom-up cache, which holds such values, is skipped.
    pub named_constants_only: bool,
}

impl<'a> SemaConstEval<'a> {
    /// Try to evaluate a constant expression at compile time.
    ///
    /// Returns `Some(IrConst)` for expressions that can be fully evaluated using
    /// sema state. Returns `None` for expressions that require lowering state
    /// (global addresses, runtime values, etc.).
    /// Evaluate an expression the language requires to be constant now --
    /// the condition of `__builtin_choose_expr` -- in GCC's
    /// `folding_initializer` mode: `__builtin_constant_p` answers 0 unless
    /// proven, because the choice is made before any pass could prove more
    /// (GCC 16.2 and Clang 23.1.3 select `x` in
    /// `__builtin_choose_expr(__builtin_constant_p(x), 100, x)` for a
    /// parameter `x`, even when every caller passes a constant).
    pub fn eval_required_constant(&self, expr: &Expr) -> Option<IrConst> {
        SemaConstEval {
            builtin_policy: self.builtin_policy.clone(),
            folding_initializer: true,
            ..*self
        }
        .eval_const_expr(expr)
    }

    pub fn eval_const_expr(&self, expr: &Expr) -> Option<IrConst> {
        // Memoization: if this expression's constant value was already computed
        // during the bottom-up sema walk, return it in O(1).
        if let Some(cache) = self.const_values {
            if let Some(cached) = cache.get(&expr.id()) {
                return Some(*cached);
            }
        }

        match expr {
            // Literals: delegate to shared evaluation
            Expr::IntLiteral(..)
            | Expr::LongLiteral(..)
            | Expr::LongLongLiteral(..)
            | Expr::UIntLiteral(..)
            | Expr::ULongLiteral(..)
            | Expr::ULongLongLiteral(..)
            | Expr::CharLiteral(..)
            | Expr::FloatLiteral(..)
            | Expr::FloatLiteralF32(..)
            | Expr::FloatLiteralLongDouble(..)
            | Expr::FloatLiteralF128(..)
            | Expr::FloatLiteralHalf(..) => shared_const_eval::eval_literal(expr),

            Expr::UnaryOp(UnaryOp::Plus, inner, _) => self.eval_const_expr(inner),
            // Of a real operand, `__real__` is the value and `__imag__` a zero
            // of its type (GCC 16.2 folds both; complex constants are not
            // modelled here).
            Expr::UnaryOp(op @ (UnaryOp::RealPart | UnaryOp::ImagPart), inner, _) => {
                let val = self.eval_const_expr(inner)?;
                if *op == UnaryOp::RealPart {
                    return Some(val);
                }
                Some(match val {
                    IrConst::F32(_) => IrConst::F32(0.0),
                    IrConst::F64(_) => IrConst::F64(0.0),
                    IrConst::I8(_) => IrConst::I8(0),
                    IrConst::I16(_) => IrConst::I16(0),
                    IrConst::I32(_) => IrConst::I32(0),
                    IrConst::I64(_) => IrConst::I64(0),
                    IrConst::I128(_) => IrConst::I128(0),
                    IrConst::Zero => IrConst::Zero,
                    IrConst::LongDouble(..) | IrConst::D32(_) | IrConst::D64(_) => return None,
                })
            }
            Expr::UnaryOp(UnaryOp::Neg, inner, _) => {
                let val = self.eval_const_expr(inner)?;
                let inner_ct = self.const_ctype(inner);
                if matches!(inner_ct, Some(CType::Float16 | CType::BFloat16)) {
                    let bits = shared_const_eval::half_bits_of(&val)?;
                    return Some(IrConst::I16((bits ^ 0x8000) as i16));
                }
                if matches!(inner_ct, Some(CType::Float128)) {
                    // Negation flips the binary128 sign bit (-0.0Q, -NaN).
                    let IrConst::I128(bits) = val else {
                        return None;
                    };
                    return Some(IrConst::I128(bits ^ i128::MIN));
                }
                let is_unsigned = self.is_expr_unsigned(inner);
                // C11 6.5.3.3p1 + 6.3.1.3p2: `-x` has the operand's promoted
                // type, and an unsigned result reduces modulo 2^width of that
                // type. The rule (including the 64-bit-carrier reduction)
                // lives in the shared folder so the sema and lowering
                // evaluators cannot drift: they used to ask the same
                // question two different ways.
                shared_const_eval::negate_const_promoted(
                    val,
                    is_unsigned,
                    self.operand_size_for_unary(inner),
                )
            }
            Expr::UnaryOp(UnaryOp::BitNot, inner, _) => {
                let val = self.eval_const_expr(inner)?;
                let is_unsigned = self.is_expr_unsigned(inner);
                // C11 6.5.3.3p4: `~x` likewise has the operand's promoted
                // type, so `~0u` is 0xFFFFFFFF (I64 carrier) and not
                // 0xFFFFFFFF_00000000, which used to flip a short-circuit
                // use (regress_logical_and_const_bitnot_short_circuit).
                // Single definition, shared with the lowering folder.
                shared_const_eval::bitnot_const_promoted(
                    val,
                    is_unsigned,
                    self.operand_size_for_unary(inner),
                )
            }

            // Logical NOT
            Expr::UnaryOp(UnaryOp::LogicalNot, inner, _) => {
                if let Some(val) = self.eval_const_expr(inner) {
                    Some(IrConst::I64(if self.const_truth(&val, inner) {
                        0
                    } else {
                        1
                    }))
                } else if Self::expr_is_always_nonzero(inner) {
                    Some(IrConst::I64(0))
                } else {
                    None
                }
            }

            // Binary operations
            Expr::BinaryOp(op, lhs, rhs, _) => {
                let l = self.eval_const_expr(lhs);
                let r = self.eval_const_expr(rhs);
                if let (Some(lv), Some(rv)) = (&l, &r) {
                    // Derive operand CTypes for signedness/width determination.
                    let lhs_ctype = self
                        .lookup_expr_type(lhs)
                        .or_else(|| self.infer_expr_ctype(lhs))
                        .or_else(|| Self::ctype_from_ir_const(lv));
                    let rhs_ctype = self
                        .lookup_expr_type(rhs)
                        .or_else(|| self.infer_expr_ctype(rhs))
                        .or_else(|| Self::ctype_from_ir_const(rv));
                    // Pass the UNPROMOTED size/signedness: the shared helper
                    // performs C11 6.3.1.1 integer promotion (sub-int types,
                    // including unsigned ones, become signed int). Clamping
                    // the size to 4 here while forwarding `ct.is_unsigned()`
                    // mis-typed `(uint8_t)201 | (int16_t)-1` as unsigned int.
                    let lhs_size = lhs_ctype.as_ref().map_or(4, |ct| self.ctype_size(ct));
                    let lhs_unsigned = lhs_ctype.as_ref().is_some_and(|ct| ct.is_unsigned());
                    let rhs_size = rhs_ctype.as_ref().map_or(4, |ct| self.ctype_size(ct));
                    let rhs_unsigned = rhs_ctype.as_ref().is_some_and(|ct| ct.is_unsigned());
                    // F33: _Float128 constants ride the I128 carrier, so the
                    // shared folders would compute on their BITS. A binop
                    // with a _Float128 side folds exactly in binary128 (the
                    // common type, C23 6.3.1.8); anything it cannot fold
                    // (pointer operands, constraint violations) declines.
                    // Sema never produces D32/D64 constants.
                    let wide_side = |ct: &Option<CType>| matches!(ct, Some(CType::Float128));
                    if wide_side(&lhs_ctype) || wide_side(&rhs_ctype) {
                        let l128 = shared_const_eval::f128_bytes_of(lv, lhs_ctype.as_ref())?;
                        let r128 = shared_const_eval::f128_bytes_of(rv, rhs_ctype.as_ref())?;
                        return shared_const_eval::fold_f128_binop(op, &l128, &r128);
                    }
                    let half_side =
                        |ct: &Option<CType>| ct.as_ref().is_some_and(CType::is_half_float);
                    if half_side(&lhs_ctype) || half_side(&rhs_ctype) {
                        if let Some(res) = shared_const_eval::fold_half_binop(
                            op,
                            lv,
                            lhs_ctype.as_ref(),
                            rv,
                            rhs_ctype.as_ref(),
                        ) {
                            return Some(res);
                        }
                        // Mixed with float/double/long double: widen the half side(s) to exact f64/f32!
                        let widen = |v: &IrConst, ct: &Option<CType>| -> Option<IrConst> {
                            match ct {
                                Some(CType::Float16) => {
                                    Some(IrConst::F32(crate::common::half_float::f16_bits_to_f32(
                                        shared_const_eval::half_bits_of(v)?,
                                    )))
                                }
                                Some(CType::BFloat16) => {
                                    Some(IrConst::F32(crate::common::half_float::bf16_bits_to_f32(
                                        shared_const_eval::half_bits_of(v)?,
                                    )))
                                }
                                _ => Some(*v),
                            }
                        };
                        let lv_w = widen(lv, &lhs_ctype)?;
                        let rv_w = widen(rv, &rhs_ctype)?;
                        return shared_const_eval::eval_binop_with_types(
                            op,
                            &lv_w,
                            &rv_w,
                            lhs_size.max(4),
                            lhs_unsigned,
                            rhs_size.max(4),
                            rhs_unsigned,
                        );
                    }
                    let result = shared_const_eval::eval_binop_with_types(
                        op,
                        lv,
                        rv,
                        lhs_size,
                        lhs_unsigned,
                        rhs_size,
                        rhs_unsigned,
                    );
                    if result.is_some() {
                        return result;
                    }
                }
                // For LogicalOr/LogicalAnd, handle string literals and other
                // always-nonzero expressions that can't be folded to numeric values.
                if *op == BinOp::LogicalOr {
                    // Sound short-circuit folding: `||` skips the RHS only when
                    // the LHS is truthy. Folding to 1 via a truthy RHS while the
                    // LHS is UNKNOWN drops the LHS's side effects (e.g.
                    // `(*q = 10) || 60`); only the LHS-side short-circuit (or a
                    // known-pure LHS constant) permits folding.
                    let l_nonzero = l.as_ref().is_some_and(|v| self.const_truth(v, lhs))
                        || Self::expr_is_always_nonzero(lhs);
                    let r_nonzero = r.as_ref().is_some_and(|v| self.const_truth(v, rhs))
                        || Self::expr_is_always_nonzero(rhs);
                    if l_nonzero || (r_nonzero && l.is_some()) {
                        return Some(IrConst::I64(1));
                    }
                    if l.as_ref().is_some_and(|v| !self.const_truth(v, lhs))
                        && r.as_ref().is_some_and(|v| !self.const_truth(v, rhs))
                    {
                        return Some(IrConst::I64(0));
                    }
                }
                if *op == BinOp::LogicalAnd {
                    // Sound short-circuit folding: `&&` skips the RHS only when
                    // the LHS is falsy. Folding to 0 via a falsy RHS while the
                    // LHS is UNKNOWN drops the LHS's side effects; only the
                    // LHS-side short-circuit (or a known-pure LHS constant) is
                    // safe to fold.
                    if l.as_ref().is_some_and(|v| !self.const_truth(v, lhs))
                        || (r.as_ref().is_some_and(|v| !self.const_truth(v, rhs)) && l.is_some())
                    {
                        return Some(IrConst::I64(0));
                    }
                    let l_nonzero = l.as_ref().is_some_and(|v| self.const_truth(v, lhs))
                        || Self::expr_is_always_nonzero(lhs);
                    let r_nonzero = r.as_ref().is_some_and(|v| self.const_truth(v, rhs))
                        || Self::expr_is_always_nonzero(rhs);
                    if l_nonzero && r_nonzero {
                        return Some(IrConst::I64(1));
                    }
                }
                None
            }

            // Cast expressions with proper bit-width tracking
            Expr::Cast(target_type, inner, _, _) => {
                let target_ctype = self.type_spec_to_ctype(target_type);
                let src_val = self.eval_const_expr(inner)?;

                // _Float128 on either side converts by value (its I128
                // carrier is not an integer: `(int)2.75Q` is 2, and
                // `(_Float128)3` is binary128 3.0, not the bit pattern 3).
                let src_ctype = self.const_ctype(inner);
                if target_ctype.is_half_float()
                    || src_ctype.as_ref().is_some_and(CType::is_half_float)
                {
                    return shared_const_eval::cast_half_const(
                        &src_val,
                        src_ctype.as_ref(),
                        &target_ctype,
                    );
                }
                if target_ctype == CType::Float128 {
                    let bytes = shared_const_eval::f128_bytes_of(&src_val, src_ctype.as_ref())?;
                    return Some(IrConst::I128(u128::from_le_bytes(bytes) as i128));
                }
                if matches!(src_ctype, Some(CType::Float128)) {
                    let IrConst::I128(bits) = src_val else {
                        return None;
                    };
                    return shared_const_eval::f128_to_ctype(
                        &(bits as u128).to_le_bytes(),
                        &target_ctype,
                    );
                }

                // Handle float source types: use value-based conversion.
                // A long double converts from its value in the target format
                // (x87: 64-bit significand), never from its f64 shadow.
                if let IrConst::LongDouble(_, bytes) = &src_val {
                    let value = crate::common::long_double::long_double_value_bytes(bytes);
                    return shared_const_eval::f128_to_ctype(&value, &target_ctype);
                }
                // float/double are exact in binary128, so one converter
                // serves every binary floating source (target-width `long`,
                // target-signedness `char`, single rounding to `float`).
                if let IrConst::F32(_) | IrConst::F64(_) = src_val {
                    let fv = src_val.to_f64()?;
                    let value = crate::common::long_double::f64_to_f128_bytes_lossless(fv);
                    return shared_const_eval::f128_to_ctype(&value, &target_ctype);
                }

                // Handle I128 source: use full 128-bit value to avoid truncation
                // through the u64-based eval_const_expr_as_bits path
                if let IrConst::I128(v128) = src_val {
                    let target_size = self.ctype_size(&target_ctype);
                    // Determine source signedness for int-to-float conversions
                    let src_unsigned = self
                        .lookup_expr_type(inner)
                        .is_some_and(|ct| ct.is_unsigned());
                    return self.cast_i128_to_ctype(v128, &target_ctype, target_size, src_unsigned);
                }

                // Integer source: use bit-based cast chain evaluation
                let (bits, _src_signed) = self.eval_const_expr_as_bits(inner)?;
                let target_size = self.ctype_size(&target_ctype);
                let target_width = target_size * 8;
                let target_signed = !target_ctype.is_unsigned() && !target_ctype.is_pointer_like();

                // For 128-bit targets, we need to sign-extend or zero-extend from 64 bits
                // based on the *source* expression's signedness, not the target's.
                // E.g., (unsigned __int128)(-1) should be all-ones (sign-extend from signed source),
                // but (unsigned __int128)(0xFFFFFFFFFFFFFFFFULL) should be 0x0000...FFFF (zero-extend).
                if target_size == 16 && !matches!(&target_ctype, CType::LongDouble) {
                    // Determine source signedness: try type map first, then infer, default signed
                    let src_signed = self
                        .lookup_expr_type(inner)
                        .or_else(|| self.infer_expr_ctype(inner))
                        .is_none_or(|ct| !ct.is_unsigned());
                    let v128 = if src_signed {
                        // Sign-extend: u64 -> i64 (reinterpret) -> i128 (sign-extend)
                        (bits as i64) as i128
                    } else {
                        // Zero-extend: u64 -> i128
                        bits as i128
                    };
                    return Some(IrConst::I128(v128));
                }

                // _Bool conversion is by VALUE (C11 6.3.1.2: nonzero -> 1),
                // never by bit truncation: it must see the FULL source
                // bits.  Truncating to 8 bits first folds (_Bool)256,
                // (_Bool)65536 and (_Bool)67106816ULL to 0 -- the dual of
                // the (_Bool)61-stays-61 bug fixed in bits_to_irconst,
                // whose normalization never sees the bits this truncation
                // already dropped.  Found by the resurrected YARPGen
                // engine (seed 20261023; regress_const_bool_cast_zero_lowbyte).
                if matches!(target_ctype, CType::Bool) {
                    return Some(shared_const_eval::c_bool_const(bits));
                }

                // Truncate to target width
                let truncated = if target_width >= 64 {
                    bits
                } else if target_width == 0 {
                    return None; // void cast
                } else {
                    bits & ((1u64 << target_width) - 1)
                };

                // For int-to-float conversions, we need the *source* signedness
                // (not target) to correctly interpret the bit pattern. E.g.,
                // (double)(unsigned long)0xFFFFFFFFFFFFFFFF should produce
                // 18446744073709551615.0, not -1.0.
                if matches!(
                    target_ctype,
                    CType::Float | CType::Double | CType::LongDouble
                ) {
                    let src_signed = self
                        .lookup_expr_type(inner)
                        .or_else(|| self.infer_expr_ctype(inner))
                        .is_none_or(|ct| !ct.is_unsigned());
                    return self.bits_to_irconst(truncated, &target_ctype, src_signed);
                }

                // Convert to IrConst based on target CType
                self.bits_to_irconst(truncated, &target_ctype, target_signed)
            }

            // A member of a C23 `constexpr` structure or union object (or a
            // `(constexpr T){...}` compound literal), reached by `.`, is a named constant.
            Expr::MemberAccess(..) => {
                let mut path = String::new();
                let mut cur = expr;
                while let Expr::MemberAccess(base, field, _) = cur {
                    path.insert_str(0, field);
                    path.insert(0, '.');
                    cur = base;
                }
                match cur {
                    Expr::Identifier(name, _) => {
                        Some(self.symbols.constexpr_member(name, &path)?.to_ir_const())
                    }
                    Expr::CompoundLiteral(ts, init, _, st) if st.is_constexpr => {
                        if let Some(val) = self
                            .types
                            .constexpr_compound_literals
                            .get(&(crate::frontend::parser::ast::ExprId::of(cur), path.clone()))
                        {
                            return Some(val.to_ir_const());
                        }
                        self.eval_constexpr_cl_path(ts, init, &path)
                    }
                    _ => None,
                }
            }

            Expr::CompoundLiteral(ts, init, _, st) if st.is_constexpr => {
                if let Some(val) = self.types.constexpr_compound_literals.get(&(
                    crate::frontend::parser::ast::ExprId::of(expr),
                    String::new(),
                )) {
                    return Some(val.to_ir_const());
                }
                self.eval_constexpr_cl_path(ts, init, "")
            }

            Expr::BitCast(target_spec, src_expr, _) => {
                self.eval_bit_cast_const(target_spec, src_expr)
            }

            // Identifier: enum constants
            Expr::Identifier(name, _) => {
                // Objects and enumerators share the ordinary-identifier
                // namespace (C11 6.2.3), so the innermost visible declaration
                // decides. A folded const object's value comes first; any other
                // object, function or typedef hides an enumerator of the same
                // name (`enum { A = 1 }; void f(void) { int A = 5;
                // _Static_assert(A, ""); }` is not constant: GCC 16.2).
                if let Some(val) = self
                    .symbols
                    .folded_value(name)
                    .filter(|_| !self.named_constants_only || self.symbols.is_constexpr(name))
                {
                    return Some(val.to_ir_const());
                }
                if self
                    .symbols
                    .lookup_ordinary(name)
                    .is_some_and(|(_, enumerator)| !enumerator)
                {
                    return None;
                }
                self.types
                    .enum_constants
                    .get(name)
                    .map(|constant| IrConst::I64(constant.value))
            }

            // sizeof
            Expr::Sizeof(arg, _) => {
                let size = match arg.as_ref() {
                    SizeofArg::Type(ts) => self.sizeof_type_spec(ts),
                    SizeofArg::Expr(e) => self.sizeof_expr(e),
                };
                size.map(|s| IrConst::I64(s as i64))
            }

            // _Countof (C2y 6.5.3.5 / GCC 16)
            Expr::Countof(arg, _) => {
                let count = match arg.as_ref() {
                    SizeofArg::Type(ts) => self.countof_type_spec(ts),
                    SizeofArg::Expr(e) => self.countof_expr(e),
                };
                count.map(|c| IrConst::I64(c as i64))
            }

            // _Alignof - C11 standard: GCC's min_align_of_type (TYPE_ALIGN
            // capped at BIGGEST_ALIGNMENT for non-user-aligned types).
            Expr::Alignof(ts, _) => {
                let align = self.c11_alignof_type_spec(ts);
                Some(IrConst::I64(align as i64))
            }

            // __alignof(type) / __alignof__(type) - GCC extension, returns preferred alignment.
            // On i686: __alignof__(long long) == 8, _Alignof(long long) == 4.
            Expr::GnuAlignof(ts, _) => {
                let align = self.preferred_alignof_type_spec(ts);
                Some(IrConst::I64(align as i64))
            }

            // __alignof__(expr) / _Alignof(expr) - alignment of expression's type.
            Expr::AlignofExpr(inner_expr, _) => self.eval_alignof_expr_const(inner_expr, false),
            Expr::GnuAlignofExpr(inner_expr, _) => self.eval_alignof_expr_const(inner_expr, true),

            // A generic selection is a constant expression when the selected
            // association is (C11 6.5.1.1p4); the controlling expression is
            // not evaluated. Same selection rule as the type checker.
            Expr::GenericSelection(controlling, associations, _) => {
                let index = self
                    .type_checker()
                    .generic_selection_index(controlling, associations)?;
                self.eval_const_expr(&associations[index].expr)
            }

            // Ternary conditional
            Expr::Conditional(cond, then_e, else_e, _) => {
                let cond_val = self.eval_const_expr(cond)?;
                if self.const_truth(&cond_val, cond) {
                    self.eval_const_expr(then_e)
                } else {
                    self.eval_const_expr(else_e)
                }
            }

            // GNU conditional (a ?: b)
            Expr::GnuConditional(cond, else_e, _) => {
                let cond_val = self.eval_const_expr(cond)?;
                if self.const_truth(&cond_val, cond) {
                    Some(cond_val)
                } else {
                    self.eval_const_expr(else_e)
                }
            }

            // __builtin_types_compatible_p
            Expr::BuiltinTypesCompatibleP(type1, type2, qualifiers1, qualifiers2, _) => {
                let ctype1 = self.type_spec_to_ctype(type1);
                let ctype2 = self.type_spec_to_ctype(type2);
                fn qualifier_levels(ty: &CType) -> usize {
                    match ty {
                        CType::Pointer(inner, _) | CType::Array(inner, _) => {
                            1 + qualifier_levels(inner)
                        }
                        _ => 1,
                    }
                }
                // `CCC_DEBUG_TYPES_COMPATIBLE=1` explains every answer: the two
                // type names as parsed, the two CType conversions, the qualifier
                // levels and the two independent halves of the verdict.  A wrong
                // answer here is a silently wrong constant in code that is
                // *specified* by it, so the reasoning must be inspectable.
                if std::env::var_os("CCC_DEBUG_TYPES_COMPATIBLE").is_some() {
                    eprintln!(
                        "[types-compatible] {t1:?} vs {t2:?}: quals {q1:?}/{q2:?} \
                         levels {l1}/{l2} types_equal={teq} quals_compatible={qok}",
                        t1 = ctype1,
                        t2 = ctype2,
                        q1 = qualifiers1,
                        q2 = qualifiers2,
                        l1 = qualifier_levels(&ctype1),
                        l2 = qualifier_levels(&ctype2),
                        teq = self.ctypes_compatible(&ctype1, &ctype2),
                        qok = crate::frontend::parser::ast::nested_type_qualifiers_compatible(
                            qualifiers1,
                            qualifier_levels(&ctype1),
                            qualifiers2,
                            qualifier_levels(&ctype2),
                        ),
                    );
                }
                let compatible = crate::frontend::parser::ast::nested_type_qualifiers_compatible(
                    qualifiers1,
                    qualifier_levels(&ctype1),
                    qualifiers2,
                    qualifier_levels(&ctype2),
                ) && self.ctypes_compatible(&ctype1, &ctype2);
                Some(IrConst::I64(compatible as i64))
            }

            // AddressOf for offsetof patterns: &((type*)0)->member
            Expr::AddressOf(inner, _) => self.eval_offsetof_pattern(inner),

            // Handle compile-time builtin function calls in constant expressions.
            Expr::FunctionCall(func, args, _) => self.eval_call_const(func, args),

            _ => None,
        }
    }

    /// A compile-time builtin call in a constant expression; `None` for any
    /// other call (and for a builtin whose value is not yet known).
    fn eval_call_const(&self, func: &Expr, args: &[Expr]) -> Option<IrConst> {
        let Expr::Identifier(name, _) = func else {
            return None;
        };
        match name.as_str() {
            "__builtin_classify_type" => {
                // The argument is not evaluated; a function designator or
                // array decays (`CType::gcc_type_class`).
                let class = match args.first() {
                    Some(arg) => self.infer_expr_ctype(arg)?.gcc_type_class(),
                    None => 0,
                };
                Some(IrConst::I64(class))
            }
            "__builtin_choose_expr" if args.len() >= 3 => {
                let cond = self.eval_required_constant(&args[0])?;
                self.eval_const_expr(if cond.is_nonzero() {
                    &args[1]
                } else {
                    &args[2]
                })
            }
            "__builtin_constant_p" => self.eval_builtin_constant_p(args.first()?),
            "__builtin_assoc_barrier" if args.len() == 1 => self.eval_const_expr(&args[0]),
            "__builtin_counted_by_ref" if args.len() == 1 => {
                if self
                    .type_checker()
                    .infer_counted_by_target(&args[0])
                    .is_none()
                {
                    Some(IrConst::I64(0))
                } else {
                    None
                }
            }
            name if crate::frontend::sema::stdbit::is_stdc_bit_builtin(name) => {
                self.eval_stdc_bit_const(name, args)
            }
            name if crate::frontend::sema::gcc16_builtins::is_gcc16_generic_bit_builtin(name) => {
                self.eval_gcc16_generic_bit_const(name, args)
            }
            name if crate::frontend::sema::gcc16_builtins::is_gcc16_bswap_bitrev_builtin(name) => {
                self.eval_gcc16_bswap_bitrev_const(name, args)
            }
            _ => {
                if !self.fold_as_library_function(name) {
                    return None;
                }
                let class_arg = if name == "__builtin_fpclassify" {
                    args.get(5)
                } else {
                    args.first()
                };
                if let Some(arg) = class_arg
                    && let Some(ct) = self.const_ctype(arg)
                    && ct.is_half_float()
                    && let Some(res) = shared_const_eval::eval_half_fpclassify_builtin(
                        name.as_str(),
                        args,
                        &ct,
                        &|e| self.eval_const_expr(e),
                    )
                {
                    return Some(res);
                }
                shared_const_eval::eval_builtin_call_with_mode(
                    name.as_str(),
                    args,
                    &|e| {
                        let v = self.eval_const_expr(e)?;
                        let ct = self.const_ctype(e);
                        Some(shared_const_eval::widen_half_const_for_builtin(
                            name.as_str(),
                            v,
                            ct.as_ref(),
                        ))
                    },
                    self.builtin_policy.may_assume(name.as_str()),
                )
            }
        }
    }

    fn eval_alignof_expr_const(&self, inner_expr: &Expr, preferred: bool) -> Option<IrConst> {
        if let Expr::Identifier(name, _) = inner_expr {
            if let Some(sym) = self.symbols.lookup(name) {
                let layouts = self.types.borrow_struct_layouts();
                let natural = sym.type_align.unwrap_or_else(|| {
                    if preferred {
                        sym.ty.preferred_align_ctx(&*layouts)
                    } else {
                        sym.ty.align_ctx(&*layouts)
                    }
                });
                if sym.type_align.is_some() || sym.explicit_alignment.is_some() {
                    let explicit = sym.explicit_alignment.unwrap_or(0);
                    return Some(IrConst::I64(natural.max(explicit) as i64));
                }
            }
        }
        let ctype = self.infer_expr_ctype(inner_expr)?;
        let layouts = self.types.borrow_struct_layouts();
        let align = if preferred {
            ctype.preferred_align_ctx(&*layouts)
        } else {
            ctype.align_ctx(&*layouts)
        };
        Some(IrConst::I64(align as i64))
    }

    fn eval_stdc_bit_const(&self, name: &str, args: &[Expr]) -> Option<IrConst> {
        use crate::frontend::sema::stdbit;
        let expected_arity = stdbit::stdc_bit_arity(name)?;
        if args.len() != expected_arity {
            return None;
        }
        let arg0_ct = self
            .lookup_expr_type(&args[0])
            .or_else(|| self.infer_expr_ctype(&args[0]))?;
        let width = stdbit::stdc_bit_width_of_ctype(&arg0_ct)?;
        let c0 = self.eval_const_expr(&args[0])?;
        let raw_val = match c0 {
            IrConst::I128(v) => v as u128,
            other => other.to_u64()? as u128,
        };
        let rotate_cnt = if expected_arity == 2 {
            let c1 = self.eval_const_expr(&args[1])?;
            let arg1_unsigned = self
                .lookup_expr_type(&args[1])
                .or_else(|| self.infer_expr_ctype(&args[1]))
                .is_some_and(|ct| ct.is_unsigned());
            Some(match (c1, arg1_unsigned) {
                (IrConst::I128(v), true) => (v as u128) as i128,
                (IrConst::I128(v), false) => v,
                (other, true) => other.to_u64()? as i128,
                (other, false) => other.to_i64()? as i128,
            })
        } else {
            None
        };
        let res = stdbit::eval_stdc_bit_builtin(name, raw_val, rotate_cnt, width)?;
        if width > 64 && stdbit::stdc_bit_returns_arg_type(name) {
            Some(IrConst::I128(res as i128))
        } else {
            Some(IrConst::I64(res as i64))
        }
    }

    fn eval_gcc16_generic_bit_const(&self, name: &str, args: &[Expr]) -> Option<IrConst> {
        use crate::frontend::sema::gcc16_builtins;
        if args.is_empty()
            || args.len() > 2
            || (args.len() == 2 && !gcc16_builtins::generic_bit_allows_fallback(name))
        {
            return None;
        }
        let arg0_ct = self
            .lookup_expr_type(&args[0])
            .or_else(|| self.infer_expr_ctype(&args[0]))?
            .decayed_value_ctype();
        let width = gcc16_builtins::validate_generic_bit_arg0(name, &arg0_ct).ok()?;
        let c0 = self.eval_const_expr(&args[0])?;
        let raw_val = match c0 {
            IrConst::I128(v) => v as u128,
            other => other.to_u64()? as u128,
        };
        let fb = if args.len() == 2 {
            let arg1_ct = self
                .lookup_expr_type(&args[1])
                .or_else(|| self.infer_expr_ctype(&args[1]))?
                .decayed_value_ctype();
            gcc16_builtins::validate_generic_bit_fallback_arg(name, &arg1_ct).ok()?;
            Some(self.eval_const_expr(&args[1])?.to_i64()?)
        } else {
            None
        };
        let res = gcc16_builtins::eval_generic_bit_builtin(name, raw_val, fb, width)?;
        Some(IrConst::I64(res))
    }

    fn eval_gcc16_bswap_bitrev_const(&self, name: &str, args: &[Expr]) -> Option<IrConst> {
        use crate::frontend::sema::gcc16_builtins;
        if gcc16_builtins::rotate_builtin_info(name).is_some() {
            if args.len() != 2 {
                return None;
            }
            let v0 = self.eval_const_expr(&args[0])?.to_u64()?;
            let v1 = self.eval_const_expr(&args[1])?.to_u64()?;
            let out = gcc16_builtins::eval_fixed_rotate(name, v0, v1)?;
            return Some(IrConst::I64(out as i64));
        }
        if args.len() != 1 {
            return None;
        }
        let c0 = self.eval_const_expr(&args[0])?;
        let raw_val = match c0 {
            IrConst::I128(v) => v as u128,
            other => other.to_u64()? as u128,
        };
        let out = gcc16_builtins::eval_bswap_or_bitrev(name, raw_val)?;
        if name == "__builtin_bswap128" {
            Some(IrConst::I128(out as i128))
        } else {
            Some(IrConst::I64(out as i64))
        }
    }

    fn eval_bit_cast_const(&self, target_spec: &TypeSpecifier, src_expr: &Expr) -> Option<IrConst> {
        let dst_ct = self.type_spec_to_ctype(target_spec);
        let src_ct = self
            .lookup_expr_type(src_expr)
            .or_else(|| self.infer_expr_ctype(src_expr))?
            .decayed_value_ctype();
        let sz = {
            let layouts = self.types.borrow_struct_layouts();
            let d_sz = dst_ct.size_ctx(&*layouts);
            let s_sz = src_ct.size_ctx(&*layouts);
            if d_sz == 0 || d_sz > 16 || d_sz != s_sz {
                return None;
            }
            d_sz
        };
        let c = self.eval_const_expr(src_expr)?;
        let mut bytes = [0u8; 16];
        match (&src_ct, c) {
            (CType::Float, IrConst::F32(f)) => {
                bytes[..4].copy_from_slice(&f.to_bits().to_le_bytes())
            }
            (CType::Float, IrConst::F64(d)) => {
                bytes[..4].copy_from_slice(&(d as f32).to_bits().to_le_bytes());
            }
            (CType::Double, IrConst::F64(d)) => {
                bytes[..8].copy_from_slice(&d.to_bits().to_le_bytes())
            }
            (CType::Double, IrConst::F32(f)) => {
                bytes[..8].copy_from_slice(&(f as f64).to_bits().to_le_bytes());
            }
            (ct, IrConst::I128(v)) if ct.is_integer() => bytes.copy_from_slice(&v.to_le_bytes()),
            (ct, other) if ct.is_integer() => {
                bytes.copy_from_slice(&(other.to_u64()? as u128).to_le_bytes());
            }
            _ => return None,
        }
        match dst_ct {
            CType::Float => Some(IrConst::F32(f32::from_bits(u32::from_le_bytes([
                bytes[0], bytes[1], bytes[2], bytes[3],
            ])))),
            CType::Double => Some(IrConst::F64(f64::from_bits(u64::from_le_bytes([
                bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
            ])))),
            CType::Bool => None,
            ct if ct.is_integer() => {
                let raw = u128::from_le_bytes(bytes);
                if sz == 16 {
                    Some(IrConst::I128(raw as i128))
                } else if ct.is_signed() {
                    let shift = 64 - (sz * 8);
                    let sext = ((raw as u64 as i64) << shift) >> shift;
                    Some(IrConst::I64(sext))
                } else {
                    let mask = if sz == 8 {
                        u64::MAX
                    } else {
                        (1u64 << (sz * 8)) - 1
                    };
                    Some(IrConst::I64(((raw as u64) & mask) as i64))
                }
            }
            _ => None,
        }
    }

    fn eval_constexpr_cl_path(
        &self,
        ts: &TypeSpecifier,
        init: &crate::frontend::parser::ast::Initializer,
        path: &str,
    ) -> Option<IrConst> {
        use crate::frontend::parser::ast::{Designator, Initializer};
        let ct = self.type_spec_to_ctype(ts);
        if path.is_empty() {
            return match init {
                Initializer::Expr(e) => self.eval_const_expr(e),
                Initializer::List(items) => match items.first() {
                    Some(it) if it.designators.is_empty() => match &it.init {
                        Initializer::Expr(e) => self.eval_const_expr(e),
                        _ => None,
                    },
                    None => Some(IrConst::I64(0)),
                    _ => None,
                },
            };
        }
        let mut cur_ct = ct;
        let mut cur_init = init;
        for seg in path.trim_start_matches('.').split('.') {
            let (CType::Struct(key) | CType::Union(key)) = &cur_ct else {
                return None;
            };
            let field_info = {
                let layouts = self.types.borrow_struct_layouts();
                let layout = layouts.get(key.as_ref())?;
                let pos = layout.fields.iter().position(|f| f.name == seg)?;
                (pos, layout.fields[pos].ty.clone())
            };
            let Initializer::List(items) = cur_init else {
                return None;
            };
            let mut cursor = 0usize;
            let mut matched: Option<&Initializer> = None;
            for item in items {
                if let Some(Designator::Field(fname)) = item.designators.first() {
                    let layouts = self.types.borrow_struct_layouts();
                    if let Some(layout) = layouts.get(key.as_ref()) {
                        if let Some(p) = layout.fields.iter().position(|f| f.name == *fname) {
                            cursor = p;
                        }
                    }
                }
                if cursor == field_info.0 {
                    matched = Some(&item.init);
                }
                cursor += 1;
            }
            match matched {
                Some(next_init) => {
                    cur_ct = field_info.1;
                    cur_init = next_init;
                }
                None => return Some(IrConst::I64(0)),
            }
        }
        match cur_init {
            Initializer::Expr(e) => self.eval_const_expr(e),
            Initializer::List(items) if items.is_empty() => Some(IrConst::I64(0)),
            _ => None,
        }
    }

    /// Whether a call spelled `name` denotes the *library* function of that name,
    /// which is the only thing the shared builtin fold table speaks for.
    ///
    /// The table answers for `strlen`, `memcmp`, `abs`, ... the library function;
    /// when the identifier denotes something else that shares the basename, the
    /// folded value and the emitted call disagree, and the disagreement is silent
    /// (measured: a file-scope `size_t (*strlen)(const char *)` object turned
    /// `strlen("abc")` into the constant 3 instead of a call through the object,
    /// and a GNU nested function named `strlen` was folded away entirely; gcc
    /// 14.2 calls the object and the nested function respectively).
    ///
    /// By what the innermost visible declaration of `name` is:
    ///
    /// * nothing visible: an ordinary library call, so fold.
    /// * a function: fold only at file scope (`lookup_depth == 0`). A block-scope
    ///   function declaration of a library name is either a redundant prototype
    ///   (folding is harmless) or a GNU nested function *definition* (folding is
    ///   wrong). Telling them apart means scanning `functions` for mangled
    ///   `parent.name` keys, which is quadratic in the number of functions, so
    ///   the depth rule declines both -- the safe answer, and the one that
    ///   matches gcc on every measured cell of the shadowing matrix.
    /// * anything else (an object, a typedef, an enumerator): that declaration
    ///   shadows the library name (C11 6.2.1p4), so do not fold.
    ///
    /// `__builtin_*` spellings are unaffected: a shadowing declaration carries
    /// the bare basename, so the lookup below never matches the prefixed name,
    /// and `eval_builtin_call_with_mode` keeps its own rule that the compiler's
    /// interface stays foldable when bare spellings are withdrawn.
    fn fold_as_library_function(&self, name: &str) -> bool {
        library_call_is_foldable(
            self.symbols
                .lookup(name)
                .map(|symbol| matches!(symbol.ty, CType::Function(_))),
            self.symbols.lookup_depth(name),
        )
    }

    /// `__builtin_constant_p(arg)`: 1 once proven, 0 where the answer can no
    /// longer change, `None` while later folding may still prove it.
    fn eval_builtin_constant_p(&self, arg: &Expr) -> Option<IrConst> {
        let cast_is_nop = |target: &TypeSpecifier| {
            let ty = self.type_spec_to_ctype(target);
            matches!(ty, CType::Pointer(..))
                || (ty.is_integer() && ty.size() == crate::common::types::target_ptr_size())
        };
        if self.eval_const_expr(arg).is_some()
            || shared_const_eval::is_string_literal_address(arg, &cast_is_nop, &|e| {
                self.eval_const_expr(e)
            })
        {
            return Some(IrConst::I32(1));
        }
        // A file-scope object's value is never a compile-time
        // constant. Resolve this negative answer even in a
        // required integer-constant-expression context (global
        // initializer / __builtin_choose_expr). Parameters and
        // locals remain deferred so inlining/propagation may
        // prove them constant later.
        if self.folding_initializer
            || matches!(arg, Expr::Identifier(id, _) if self.symbols.lookup_global(id).is_some())
        {
            return Some(IrConst::I32(0));
        }
        // The same for a block-scope object with static
        // storage (a static local): GCC 16.2 answers 0 for
        // any object address, and the kernel's
        // PIN_GROUP(.mode = __builtin_choose_expr(
        // __builtin_constant_p((m)), (m), 0)) needs the 0
        // when `m` is a static-local pin array.
        if matches!(arg, Expr::Identifier(id, _) if
            self.symbols.lookup(id).is_some()
                && !self.symbols.is_automatic_object(id))
        {
            return Some(IrConst::I32(0));
        }
        None
    }

    /// Evaluate a constant expression, returning raw u64 bits and signedness.
    /// Preserves signedness through cast chains for proper widening.
    fn eval_const_expr_as_bits(&self, expr: &Expr) -> Option<(u64, bool)> {
        match expr {
            Expr::Cast(target_type, inner, _, _) => {
                let (bits, _src_signed) = self.eval_const_expr_as_bits(inner)?;
                let target_ctype = self.type_spec_to_ctype(target_type);
                // _Bool: the value is 0/1 by truthiness of the FULL
                // source bits, not by truncation to 8 bits (C11 6.3.1.2;
                // see the Cast arm in eval_const_expr).
                if matches!(target_ctype, CType::Bool) {
                    return Some((u64::from(shared_const_eval::c_bool_truth(bits)), false));
                }
                let target_width = self.ctype_size(&target_ctype) * 8;
                let target_signed = !target_ctype.is_unsigned() && !target_ctype.is_pointer_like();
                Some(const_arith::truncate_and_extend_bits(
                    bits,
                    target_width,
                    target_signed,
                ))
            }
            _ => {
                let val = self.eval_const_expr(expr)?;
                Some(shared_const_eval::irconst_to_bits(&val))
            }
        }
    }

    /// Evaluate the offsetof pattern: &((type*)0)->member
    /// Also handles nested member access like &((type*)0)->data.x
    fn eval_offsetof_pattern(&self, expr: &Expr) -> Option<IrConst> {
        let (offset, _ty) = self.eval_offsetof_pattern_with_type(expr)?;
        Some(IrConst::I64(offset as i64))
    }

    /// Evaluate an offsetof sub-expression, returning both the accumulated byte offset
    /// and the CType of the resulting expression (needed for chained member access).
    fn eval_offsetof_pattern_with_type(&self, expr: &Expr) -> Option<(usize, CType)> {
        match expr {
            Expr::PointerMemberAccess(base, field_name, _) => {
                let (layout, base_offset) = self.offsetof_base_layout(base)?;
                let (field_offset, field_ty) =
                    layout.field_offset(field_name, &*self.types.borrow_struct_layouts())?;
                Some((base_offset + field_offset, field_ty))
            }
            Expr::MemberAccess(base, field_name, _) => {
                // First try: base is *((type*)0) (deref pattern)
                if let Expr::Deref(inner, _) = base.as_ref() {
                    let (layout, base_offset) = self.offsetof_base_layout(inner)?;
                    let (field_offset, field_ty) =
                        layout.field_offset(field_name, &*self.types.borrow_struct_layouts())?;
                    return Some((base_offset + field_offset, field_ty));
                }
                // Second try: base is itself an offsetof sub-expression (chained access)
                // e.g., ((type*)0)->data.x where base = ((type*)0)->data
                let (base_offset, base_type) = self.eval_offsetof_pattern_with_type(base)?;
                let struct_key = match &base_type {
                    CType::Struct(key) | CType::Union(key) => key.clone(),
                    _ => return None,
                };
                let layouts = self.types.borrow_struct_layouts();
                let layout = layouts.get(&*struct_key)?;
                let (field_offset, field_ty) = layout.field_offset(field_name, &*layouts)?;
                Some((base_offset + field_offset, field_ty))
            }
            Expr::ArraySubscript(base, index, _) => {
                let (base_offset, base_type) = self.eval_offsetof_pattern_with_type(base)?;
                let idx_val = self.eval_const_expr(index)?;
                let idx = idx_val.to_i64()?;
                let elem_size = match &base_type {
                    CType::Array(elem, _) => elem.size_ctx(&*self.types.borrow_struct_layouts()),
                    _ => return None,
                };
                let elem_ty = match &base_type {
                    CType::Array(elem, _) => (**elem).clone(),
                    _ => return None,
                };
                Some((
                    (base_offset as i64 + idx * elem_size as i64) as usize,
                    elem_ty,
                ))
            }
            _ => None,
        }
    }

    /// The record layout and starting offset behind the null-pointer base of
    /// an offsetof pattern (`((T *)0)->m`, `(*(T *)0).m`).
    ///
    /// The record is taken from the type sema recorded for `base` when it
    /// analyzed the expression: the pointee of `->`'s operand is by definition
    /// the record accessed. That is the only way to reach an anonymous record
    /// (`offsetof(struct { char c; int i; }, i)`): its specifier has no tag,
    /// and its layout was registered under the `__anon_struct_N` key minted
    /// when the type checker resolved that very cast. The specifier is the
    /// fallback for an evaluator constructed without the annotation map.
    fn offsetof_base_layout(&self, base: &Expr) -> Option<(crate::common::types::RcLayout, usize)> {
        let (type_spec, base_offset) = self.extract_null_pointer_cast_with_offset(base)?;
        let annotated = match self.lookup_expr_type(base) {
            Some(CType::Pointer(pointee, _)) => match &*pointee {
                CType::Struct(key) | CType::Union(key) => {
                    self.types.borrow_struct_layouts().get(&**key).cloned()
                }
                _ => None,
            },
            _ => None,
        };
        let layout = match annotated {
            Some(layout) => layout,
            None => self.get_struct_layout_for_type(&type_spec)?,
        };
        Some((layout, base_offset))
    }

    /// Extract the struct type from a (type*)0 pattern.
    fn extract_null_pointer_cast_with_offset(&self, expr: &Expr) -> Option<(TypeSpecifier, usize)> {
        match expr {
            Expr::Cast(type_spec, inner, _, _) => {
                if let TypeSpecifier::Pointer(inner_ts, _) = type_spec {
                    if const_arith::is_zero_expr(inner) {
                        return Some((*inner_ts.clone(), 0));
                    }
                }
                None
            }
            _ => None,
        }
    }

    /// Get the struct layout for a type specifier.
    fn get_struct_layout_for_type(
        &self,
        type_spec: &TypeSpecifier,
    ) -> Option<crate::common::types::RcLayout> {
        let ctype = self.type_spec_to_ctype(type_spec);
        match &ctype {
            CType::Struct(key) | CType::Union(key) => {
                self.types.borrow_struct_layouts().get(&**key).cloned()
            }
            _ => None,
        }
    }

    /// Check if two CTypes are compatible (for __builtin_types_compatible_p).
    fn ctypes_compatible(&self, t1: &CType, t2: &CType) -> bool {
        // ONE definition, shared with the lowering: see
        // `crate::common::types::ctypes_compatible`.  The copy that used to live
        // here and the copy in the lowering disagreed on function types (this
        // one ignored parameter names, the lowering's did not), so the same
        // predicate answered 1 in sema and 0 in the object.
        crate::common::types::ctypes_compatible(t1, t2)
    }

    // === Type helper methods ===

    /// Look up the pre-annotated CType for an expression from sema's expr_types map.
    /// This is O(1) and preserves signedness information (e.g. unsigned int from a cast).
    fn lookup_expr_type(&self, expr: &Expr) -> Option<CType> {
        self.expr_types.and_then(|m| m.get(&expr.id()).cloned())
    }

    /// Derive a CType from an IrConst value.
    /// This is O(1) and avoids the potentially exponential infer_expr_ctype()
    /// when we already have the evaluated constant value.
    /// Note: This loses signedness info -- IrConst::I64 always maps to signed Long.
    fn ctype_from_ir_const(c: &IrConst) -> Option<CType> {
        match c {
            IrConst::I8(_) => Some(CType::Char),
            IrConst::I16(_) => Some(CType::Short),
            IrConst::I32(_) => Some(CType::Int),
            IrConst::I64(_) => Some(CType::Long),
            IrConst::I128(_) => Some(CType::Int128),
            IrConst::F32(_) => Some(CType::Float),
            IrConst::F64(_) => Some(CType::Double),
            IrConst::D32(_) => Some(CType::Decimal32),
            IrConst::D64(_) => Some(CType::Decimal64),
            IrConst::LongDouble(..) => Some(CType::LongDouble),
            IrConst::Zero => Some(CType::Int),
        }
    }

    /// Convert a TypeSpecifier to CType using sema's canonical type resolution.
    fn type_spec_to_ctype(&self, spec: &TypeSpecifier) -> CType {
        self.ctype_from_type_spec(spec)
    }

    /// Infer the CType of an expression using ExprTypeChecker.
    fn type_checker(&self) -> super::type_checker::ExprTypeChecker<'_> {
        super::type_checker::ExprTypeChecker {
            symbols: self.symbols,
            types: self.types,
            functions: self.functions,
            expr_types: self.expr_types,
            builtin_policy: self.builtin_policy.clone(),
            pre_c99: self.pre_c99,
        }
    }

    fn infer_expr_ctype(&self, expr: &Expr) -> Option<CType> {
        self.type_checker().infer_expr_ctype(expr)
    }

    /// Get the size in bytes for a CType.
    fn ctype_size(&self, ctype: &CType) -> usize {
        ctype.size_ctx(&*self.types.borrow_struct_layouts())
    }

    /// Size in bytes of `inner`'s type, when a type is known.
    ///
    /// This is the width a unary `-`/`~` fold must reduce to (C11
    /// 6.3.1.3p2). Both constant folders ask this of the same operand, so
    /// the resolution order is spelled once, here: the sema type map
    /// first, then AST-level inference -- the same order every cast path
    /// in this file uses.
    fn operand_size_for_unary(&self, inner: &Expr) -> Option<usize> {
        self.lookup_expr_type(inner)
            .or_else(|| self.infer_expr_ctype(inner))
            .map(|ct| self.ctype_size(&ct))
    }

    /// Check if an expression has an unsigned type, using the expr_types cache
    /// or falling back to type inference from the AST structure.
    fn is_expr_unsigned(&self, expr: &Expr) -> bool {
        // First try the pre-annotated type from sema
        if let Some(ctype) = self.lookup_expr_type(expr) {
            return ctype.is_unsigned();
        }
        // For cast expressions, check the target type directly
        if let Expr::Cast(target_type, _, _, _) = expr {
            let ctype = self.type_spec_to_ctype(target_type);
            return ctype.is_unsigned();
        }
        // Fall back to type inference
        if let Some(ctype) = self.infer_expr_ctype(expr) {
            return ctype.is_unsigned();
        }
        false
    }

    /// The C type of a constant operand, for value-dependent folding.
    fn const_ctype(&self, expr: &Expr) -> Option<CType> {
        self.lookup_expr_type(expr)
            .or_else(|| self.infer_expr_ctype(expr))
    }

    /// The truth value of a folded constant (C11 6.3.1.2: `!= 0`), decided by
    /// its C type: the I128 carrier of -0.0Q or -0.0DL is false, a NaN true.
    fn const_truth(&self, val: &IrConst, expr: &Expr) -> bool {
        shared_const_eval::const_truth(val, self.const_ctype(expr).as_ref())
    }

    /// Convert raw bits to an IrConst based on target CType.
    ///
    /// For unsigned sub-int types (UChar, UShort, Bool), we store the value
    /// as IrConst::I32 with zero-extension rather than I8/I16, because IrConst
    /// only has signed I8/I16 variants. Without this, `(unsigned short)(-1)` would
    /// be stored as `I16(-1)`, and `to_i64()` would sign-extend to -1 instead of
    /// 65535 -- causing `(unsigned short)(-1) << 1` to evaluate as -2 instead of
    /// 131070. This matches C's integer promotion (sub-int → int).
    fn bits_to_irconst(&self, bits: u64, target: &CType, target_signed: bool) -> Option<IrConst> {
        let size = self.ctype_size(target);
        Some(match size {
            1 => {
                if matches!(target, CType::Bool) {
                    // C11 6.3.1.2: any nonzero value converted to _Bool becomes 1.
                    // Without this, (_Bool)61 would stay 61 (stored as I32(61)) and
                    // change loop trip counts / boolean arithmetic. Bug found via
                    // Regehr yarpgen differential test regress_const_bool_cast_normalization.
                    IrConst::I32(if bits != 0 { 1 } else { 0 })
                } else if !target_signed {
                    // Unsigned char/bool: store as I32 to preserve unsigned value
                    // through integer promotion (e.g., (unsigned char)255 → I32(255), not I8(-1))
                    IrConst::I32(bits as u8 as i32)
                } else {
                    IrConst::I8(bits as i8)
                }
            }
            2 => {
                if !target_signed {
                    // Unsigned short: store as I32 to preserve unsigned value
                    // through integer promotion (e.g., (unsigned short)65535 → I32(65535), not I16(-1))
                    IrConst::I32(bits as u16 as i32)
                } else {
                    IrConst::I16(bits as i16)
                }
            }
            4 => {
                if matches!(target, CType::Float) {
                    let int_val = if target_signed {
                        bits as i64 as f32
                    } else {
                        bits as f32
                    };
                    IrConst::F32(int_val)
                } else if !target_signed {
                    // Unsigned 32-bit: use I64 to preserve unsigned semantics.
                    // IrConst::I32 is signed, so values >= 0x80000000 would sign-extend
                    // incorrectly when converted to i64 for 64-bit comparisons.
                    IrConst::I64(bits as u32 as i64)
                } else {
                    IrConst::I32(bits as i32)
                }
            }
            8 => {
                if matches!(target, CType::Double) {
                    let int_val = if target_signed {
                        bits as i64 as f64
                    } else {
                        bits as f64
                    };
                    IrConst::F64(int_val)
                } else {
                    IrConst::I64(bits as i64)
                }
            }
            16 => {
                if matches!(target, CType::LongDouble) {
                    // Use direct integer-to-x87 conversion to preserve full 64-bit
                    // precision (x87 has 64-bit mantissa, unlike f64's 52-bit).
                    if target_signed {
                        IrConst::long_double_from_i64(bits as i64)
                    } else {
                        IrConst::long_double_from_u64(bits)
                    }
                } else {
                    IrConst::I128(bits as i128) // __int128 / unsigned __int128
                }
            }
            _ => {
                // Pointer types and other 8-byte types
                if target.is_pointer_like() {
                    IrConst::I64(bits as i64)
                } else {
                    return None;
                }
            }
        })
    }

    /// Cast a full 128-bit integer value to a target CType.
    ///
    /// This handles casts from __int128/unsigned __int128 without going through the
    /// u64-based eval_const_expr_as_bits path, which would truncate the upper 64 bits.
    /// For targets <= 64 bits, extract the lower bits. For 128-bit targets, preserve
    /// the full value.
    fn cast_i128_to_ctype(
        &self,
        v128: i128,
        target: &CType,
        target_size: usize,
        src_unsigned: bool,
    ) -> Option<IrConst> {
        let bits_lo = v128 as u64; // lower 64 bits
        let target_signed = !target.is_unsigned() && !target.is_pointer_like();
        Some(match target_size {
            0 => return None, // void cast
            1 => {
                if matches!(target, CType::Bool) {
                    // C11 6.3.1.2 _Bool normalization for the I128-source
                    // cast path: truth is a property of the FULL value --
                    // (unsigned __int128)1 << 100 has zero low 64 bits but
                    // converts to _Bool 1.
                    shared_const_eval::c_bool_const_i128(v128)
                } else if !target_signed {
                    IrConst::I32(bits_lo as u8 as i32)
                } else {
                    IrConst::I8(bits_lo as i8)
                }
            }
            2 => {
                if !target_signed {
                    IrConst::I32(bits_lo as u16 as i32)
                } else {
                    IrConst::I16(bits_lo as i16)
                }
            }
            4 => {
                if matches!(target, CType::Float) {
                    // int-to-float: signedness comes from the source type
                    let fv = if src_unsigned {
                        (v128 as u128) as f32
                    } else {
                        v128 as f32
                    };
                    IrConst::F32(fv)
                } else if !target_signed {
                    IrConst::I64(bits_lo as u32 as i64)
                } else {
                    IrConst::I32(bits_lo as i32)
                }
            }
            8 => {
                if matches!(target, CType::Double) {
                    let fv = if src_unsigned {
                        (v128 as u128) as f64
                    } else {
                        v128 as f64
                    };
                    IrConst::F64(fv)
                } else {
                    IrConst::I64(bits_lo as i64)
                }
            }
            16 => {
                if matches!(target, CType::LongDouble) {
                    // Use direct integer-to-x87 conversion to preserve full 64-bit
                    // mantissa precision (x87 has 64-bit mantissa, unlike f64's 52-bit).
                    if src_unsigned {
                        IrConst::long_double_from_u128(v128 as u128)
                    } else {
                        IrConst::long_double_from_i128(v128)
                    }
                } else {
                    // __int128 / unsigned __int128: preserve full 128-bit value
                    IrConst::I128(v128)
                }
            }
            _ => {
                if target.is_pointer_like() {
                    IrConst::I64(bits_lo as i64)
                } else {
                    return None;
                }
            }
        })
    }

    /// The enum a specifier names: a body's own values, else the completed
    /// definition visible in this lexical scope (the analyzer's
    /// `resolve_enum` rule). Size, alignment and every integer property follow
    /// from it through `EnumType::compatible_ctype`.
    fn resolve_enum_type(
        &self,
        name: &Option<String>,
        variants: &Option<Vec<crate::frontend::parser::ast::EnumVariant>>,
        is_packed: bool,
        underlying: Option<&TypeSpecifier>,
    ) -> crate::common::types::EnumType {
        crate::common::type_builder::enum_type_of(
            name,
            variants,
            is_packed,
            name.as_ref().and_then(|tag| self.types.enum_types.get(tag)),
            underlying.map(|u| self.ctype_from_type_spec(u)),
            |v| {
                v.value
                    .as_ref()
                    .and_then(|e| self.eval_const_expr(e))
                    .and_then(|c| c.to_i64())
            },
        )
    }

    /// Compute sizeof for a type specifier.
    /// Returns None if the type cannot be sized (e.g., typeof(expr) without expr type info).
    fn sizeof_type_spec(&self, spec: &TypeSpecifier) -> Option<usize> {
        use crate::common::types::target_ptr_size;
        let ptr_sz = target_ptr_size();
        match spec {
            TypeSpecifier::Void => Some(0),
            TypeSpecifier::Char
            | TypeSpecifier::SignedChar
            | TypeSpecifier::UnsignedChar
            | TypeSpecifier::PlainUnsignedChar => Some(1),
            TypeSpecifier::Short | TypeSpecifier::UnsignedShort => Some(2),
            TypeSpecifier::Int
            | TypeSpecifier::UnsignedInt
            | TypeSpecifier::Signed
            | TypeSpecifier::Unsigned => Some(4),
            TypeSpecifier::Long | TypeSpecifier::UnsignedLong => Some(ptr_sz),
            TypeSpecifier::LongLong | TypeSpecifier::UnsignedLongLong => Some(8),
            TypeSpecifier::Int128 | TypeSpecifier::UnsignedInt128 => Some(16),
            TypeSpecifier::Float16 | TypeSpecifier::BFloat16 => Some(2),
            TypeSpecifier::Float => Some(4),
            TypeSpecifier::Double => Some(8),
            TypeSpecifier::LongDouble => Some(if ptr_sz == 4 { 12 } else { 16 }),
            TypeSpecifier::Bool => Some(1),
            TypeSpecifier::ComplexFloat => Some(8),
            TypeSpecifier::ComplexDouble => Some(16),
            TypeSpecifier::ComplexLongDouble => Some(if ptr_sz == 4 { 24 } else { 32 }),
            TypeSpecifier::Pointer(_, _)
            | TypeSpecifier::FunctionPointer(..)
            | TypeSpecifier::NullPtrT => Some(ptr_sz),
            TypeSpecifier::Array(elem, Some(size)) => {
                let elem_size = self.sizeof_type_spec(elem)?;
                let n = self.eval_const_expr(size)?.to_i64()?;
                Some(elem_size.saturating_mul(n as usize))
            }
            TypeSpecifier::Array(_, None) => Some(ptr_sz), // incomplete array
            TypeSpecifier::Struct(
                tag,
                fields,
                is_packed,
                pragma_pack,
                struct_aligned,
                reverse_sso,
            ) => {
                // Look up cached layout for tagged structs
                if let Some(tag) = tag {
                    // Through the alias, not the raw tag: inside a shadowing
                    // scope the tag denotes the inner record, and sizeof has to
                    // report the inner size.
                    let base = format!("struct.{}", tag);
                    let key = self.types.resolve_record_key(&base);
                    if let Some(layout) = self.types.borrow_struct_layouts().get(key.as_ref()) {
                        return Some(layout.size);
                    }
                }
                if let Some(fields) = fields {
                    let struct_fields = self.convert_struct_fields(fields);
                    if !struct_fields.is_empty() {
                        let max_field_align = if *is_packed { Some(1) } else { *pragma_pack };
                        let mut layout =
                            crate::common::types::StructLayout::for_struct_with_packing(
                                &struct_fields,
                                max_field_align,
                                &*self.types.borrow_struct_layouts(),
                                reverse_sso.is_some_and(|v| v),
                            );
                        layout.apply_struct_aligned(*struct_aligned);
                        return Some(layout.size);
                    }
                }
                Some(0)
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
                    let base = format!("union.{}", tag);
                    let key = self.types.resolve_record_key(&base);
                    if let Some(layout) = self.types.borrow_struct_layouts().get(key.as_ref()) {
                        return Some(layout.size);
                    }
                }
                if let Some(fields) = fields {
                    let union_fields = self.convert_struct_fields(fields);
                    if !union_fields.is_empty() {
                        let max_field_align = if *is_packed { Some(1) } else { *pragma_pack };
                        let mut layout = crate::common::types::StructLayout::for_union_with_packing(
                            &union_fields,
                            max_field_align,
                            &*self.types.borrow_struct_layouts(),
                            reverse_sso.is_some_and(|v| v),
                        );
                        layout.apply_struct_aligned(*struct_aligned);
                        return Some(layout.size);
                    }
                }
                Some(0)
            }
            TypeSpecifier::Enum(name, variants, is_packed, underlying) => Some(
                self.resolve_enum_type(name, variants, *is_packed, underlying.as_deref())
                    .packed_size(),
            ),
            TypeSpecifier::TypedefName(name) => {
                if let Some(ctype) = self.types.typedefs.get(name) {
                    Some(ctype.size_ctx(&*self.types.borrow_struct_layouts()))
                } else {
                    Some(8) // fallback
                }
            }
            TypeSpecifier::TypeofType(inner, _) => self.sizeof_type_spec(inner),
            TypeSpecifier::Typeof(expr, _) => {
                let ctype = self.infer_expr_ctype(expr)?;
                Some(ctype.size_ctx(&*self.types.borrow_struct_layouts()))
            }
            TypeSpecifier::Vector(_, total_bytes) => Some(*total_bytes),
            TypeSpecifier::ExtVector(inner, lanes) => {
                let element = self.ctype_from_type_spec(inner);
                ExtVectorLayout::new(&element, *lanes).map(|layout| layout.storage_size)
            }
            _ => None,
        }
    }

    /// Compute sizeof for an expression via its CType.
    ///
    /// Special cases: string literals are arrays, not pointers, inside sizeof
    /// (C11 6.3.2.1p3: array-to-pointer decay does not apply to sizeof operands).
    fn sizeof_expr(&self, expr: &Expr) -> Option<usize> {
        match expr {
            // "hello" is char[6], not char* -- sizeof gives array size.
            // Use chars().count() because high-byte escape sequences like \xff
            // are stored as multi-byte UTF-8 in the Rust String but represent
            // single bytes in C.
            Expr::StringLiteral(s, _) => return Some(s.chars().count() + 1),
            // L"hello" is wchar_t[6] (int[6] on Linux) -- sizeof gives array size
            Expr::WideStringLiteral(s, _) => return Some((s.chars().count() + 1) * 4),
            // u"hello" is char16_t[6] (unsigned short[6]) -- sizeof gives array size
            Expr::Char16StringLiteral(s, _) => return Some((s.encode_utf16().count() + 1) * 2),
            _ => {}
        }
        let ctype = self.infer_expr_ctype(expr)?;
        Some(ctype.size_ctx(&*self.types.borrow_struct_layouts()))
    }

    fn countof_type_spec(&self, spec: &TypeSpecifier) -> Option<usize> {
        if let TypeSpecifier::Array(_, Some(bound_expr)) = spec {
            return self
                .eval_const_expr(bound_expr)
                .and_then(|c| c.to_i64())
                .and_then(|n| usize::try_from(n).ok());
        }
        match self.ctype_from_type_spec(spec) {
            CType::Array(_, Some(n)) => Some(n),
            _ => None,
        }
    }

    fn countof_expr(&self, expr: &Expr) -> Option<usize> {
        match expr {
            Expr::StringLiteral(s, _) | Expr::WideStringLiteral(s, _) => {
                return Some(s.chars().count() + 1);
            }
            Expr::Char16StringLiteral(s, _) => return Some(s.encode_utf16().count() + 1),
            Expr::Comma(..) | Expr::Conditional(..) | Expr::GnuConditional(..) => return None,
            Expr::CompoundLiteral(TypeSpecifier::Array(_, None), init, _, _) => {
                return match init.as_ref() {
                    crate::frontend::parser::ast::Initializer::List(items)
                        if items.iter().all(|it| it.designators.is_empty()) =>
                    {
                        Some(items.len())
                    }
                    crate::frontend::parser::ast::Initializer::Expr(
                        Expr::StringLiteral(s, _) | Expr::WideStringLiteral(s, _),
                    ) => Some(s.chars().count() + 1),
                    crate::frontend::parser::ast::Initializer::Expr(Expr::Char16StringLiteral(
                        s,
                        _,
                    )) => Some(s.encode_utf16().count() + 1),
                    _ => None,
                };
            }
            _ => {}
        }
        match self.infer_expr_ctype(expr)? {
            CType::Array(_, Some(n)) => Some(n),
            _ => None,
        }
    }

    /// Compute alignof for a type specifier.
    /// C11 `_Alignof(type-name)`: `alignof_type_spec` (GCC TYPE_ALIGN) capped
    /// at `BIGGEST_ALIGNMENT` unless the type is user-aligned — GCC's
    /// `min_align_of_type`; see `CType::c11_alignof_ctx`. Mirrors
    /// `FunctionLowering::c11_alignof_type`.
    fn c11_alignof_type_spec(&self, spec: &TypeSpecifier) -> usize {
        let align = self.alignof_type_spec(spec);
        if align <= crate::common::types::target_biggest_alignment() {
            return align;
        }
        if let TypeSpecifier::TypedefName(name) = spec {
            if self.types.typedef_alignments.contains_key(name) {
                return align;
            }
        }
        let ctype = self.ctype_from_type_spec(spec);
        align.min(ctype.c11_alignof_ctx(&*self.types.borrow_struct_layouts()))
    }

    fn alignof_type_spec(&self, spec: &TypeSpecifier) -> usize {
        use crate::common::types::target_ptr_size;
        let ptr_sz = target_ptr_size();
        match spec {
            TypeSpecifier::Void | TypeSpecifier::Bool => 1,
            TypeSpecifier::Char
            | TypeSpecifier::SignedChar
            | TypeSpecifier::UnsignedChar
            | TypeSpecifier::PlainUnsignedChar => 1,
            TypeSpecifier::Short | TypeSpecifier::UnsignedShort => 2,
            TypeSpecifier::Int
            | TypeSpecifier::UnsignedInt
            | TypeSpecifier::Signed
            | TypeSpecifier::Unsigned => 4,
            TypeSpecifier::Long | TypeSpecifier::UnsignedLong => ptr_sz,
            TypeSpecifier::LongLong | TypeSpecifier::UnsignedLongLong => {
                if ptr_sz == 4 {
                    4
                } else {
                    8
                }
            }
            TypeSpecifier::Int128 | TypeSpecifier::UnsignedInt128 => 16,
            TypeSpecifier::Float16 | TypeSpecifier::BFloat16 => 2,
            TypeSpecifier::Float => 4,
            TypeSpecifier::Double => {
                if ptr_sz == 4 {
                    4
                } else {
                    8
                }
            }
            // On i686, long double is 80-bit x87 aligned to 4 bytes
            TypeSpecifier::LongDouble => {
                if ptr_sz == 4 {
                    4
                } else {
                    16
                }
            }
            TypeSpecifier::ComplexFloat => 4,
            TypeSpecifier::ComplexDouble => {
                if ptr_sz == 4 {
                    4
                } else {
                    8
                }
            }
            TypeSpecifier::ComplexLongDouble => {
                if ptr_sz == 4 {
                    4
                } else {
                    16
                }
            }
            TypeSpecifier::Pointer(_, _)
            | TypeSpecifier::FunctionPointer(..)
            | TypeSpecifier::NullPtrT => ptr_sz,
            TypeSpecifier::Array(elem, _) => self.alignof_type_spec(elem),
            TypeSpecifier::Struct(
                tag,
                fields,
                is_packed,
                pragma_pack,
                struct_aligned,
                reverse_sso,
            ) => {
                if let Some(tag) = tag {
                    let base = format!("struct.{}", tag);
                    let key = self.types.resolve_record_key(&base);
                    if let Some(layout) = self.types.borrow_struct_layouts().get(key.as_ref()) {
                        return layout.align;
                    }
                }
                if let Some(fields) = fields {
                    let struct_fields = self.convert_struct_fields(fields);
                    if !struct_fields.is_empty() {
                        let max_field_align = if *is_packed { Some(1) } else { *pragma_pack };
                        let mut layout =
                            crate::common::types::StructLayout::for_struct_with_packing(
                                &struct_fields,
                                max_field_align,
                                &*self.types.borrow_struct_layouts(),
                                reverse_sso.is_some_and(|v| v),
                            );
                        layout.apply_struct_aligned(*struct_aligned);
                        return layout.align;
                    }
                }
                struct_aligned.unwrap_or(1)
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
                    let base = format!("union.{}", tag);
                    let key = self.types.resolve_record_key(&base);
                    if let Some(layout) = self.types.borrow_struct_layouts().get(key.as_ref()) {
                        return layout.align;
                    }
                }
                if let Some(fields) = fields {
                    let union_fields = self.convert_struct_fields(fields);
                    if !union_fields.is_empty() {
                        let max_field_align = if *is_packed { Some(1) } else { *pragma_pack };
                        let mut layout = crate::common::types::StructLayout::for_union_with_packing(
                            &union_fields,
                            max_field_align,
                            &*self.types.borrow_struct_layouts(),
                            reverse_sso.is_some_and(|v| v),
                        );
                        layout.apply_struct_aligned(*struct_aligned);
                        return layout.align;
                    }
                }
                struct_aligned.unwrap_or(1)
            }
            TypeSpecifier::Enum(name, variants, is_packed, underlying) => {
                let enum_type = CType::Enum(self.resolve_enum_type(
                    name,
                    variants,
                    *is_packed,
                    underlying.as_deref(),
                ));
                enum_type.align_ctx(&*self.types.borrow_struct_layouts())
            }
            TypeSpecifier::TypedefName(name) => {
                // A typedef-level `aligned(N)` replaces the alignment (may
                // lower it) — same rule as `FunctionLowering::alignof_type`.
                if let Some(&td_align) = self.types.typedef_alignments.get(name) {
                    td_align
                } else if let Some(ctype) = self.types.typedefs.get(name) {
                    ctype.align_ctx(&*self.types.borrow_struct_layouts())
                } else {
                    crate::common::types::target_ptr_size()
                }
            }
            TypeSpecifier::TypeofType(inner, _) => self.alignof_type_spec(inner),
            TypeSpecifier::Typeof(expr, _) => {
                if let Some(ctype) = self.infer_expr_ctype(expr) {
                    ctype.align_ctx(&*self.types.borrow_struct_layouts())
                } else {
                    crate::common::types::target_ptr_size() // fallback
                }
            }
            // GCC TYPE_ALIGN of a vector (full size on x86); the C11
            // _Alignof cap lives in `c11_alignof_type_spec`.
            TypeSpecifier::Vector(_, total_bytes) => {
                crate::common::types::vector_type_align(*total_bytes)
            }
            TypeSpecifier::ExtVector(inner, lanes) => {
                let element = self.ctype_from_type_spec(inner);
                ExtVectorLayout::new(&element, *lanes)
                    .map_or(crate::common::types::target_ptr_size(), |layout| {
                        layout.alignment
                    })
            }
            _ => crate::common::types::target_ptr_size(),
        }
    }

    /// Compute preferred (natural) alignment for a type specifier.
    /// Used by GCC's __alignof/__alignof__ which returns preferred alignment.
    /// On i686: __alignof__(long long) == 8, __alignof__(double) == 8.
    fn preferred_alignof_type_spec(&self, spec: &TypeSpecifier) -> usize {
        use crate::common::types::target_ptr_size;
        let ptr_sz = target_ptr_size();
        if ptr_sz != 4 {
            return self.alignof_type_spec(spec);
        }
        // On i686: long long and double have preferred alignment of 8
        match spec {
            TypeSpecifier::LongLong | TypeSpecifier::UnsignedLongLong => 8,
            TypeSpecifier::Double => 8,
            TypeSpecifier::ComplexDouble => 8,
            TypeSpecifier::Array(elem, _) => self.preferred_alignof_type_spec(elem),
            TypeSpecifier::TypedefName(name) => {
                // Typedef `aligned(N)` replaces the alignment (may lower it).
                if let Some(&td_align) = self.types.typedef_alignments.get(name) {
                    td_align
                } else if let Some(ctype) = self.types.typedefs.get(name) {
                    ctype.preferred_align_ctx(&*self.types.borrow_struct_layouts())
                } else {
                    target_ptr_size()
                }
            }
            TypeSpecifier::TypeofType(inner, _) => self.preferred_alignof_type_spec(inner),
            // An 8-byte enum prefers 8 like `long long` (GCC `__alignof__`, -m32).
            TypeSpecifier::Enum(name, variants, is_packed, underlying) => CType::Enum(
                self.resolve_enum_type(name, variants, *is_packed, underlying.as_deref()),
            )
            .preferred_align_ctx(&*self.types.borrow_struct_layouts()),
            TypeSpecifier::Typeof(expr, _) => {
                if let Some(ctype) = self.infer_expr_ctype(expr) {
                    ctype.preferred_align_ctx(&*self.types.borrow_struct_layouts())
                } else {
                    target_ptr_size()
                }
            }
            TypeSpecifier::Vector(_, total_bytes) => {
                crate::common::types::vector_type_align(*total_bytes)
            }
            TypeSpecifier::ExtVector(inner, lanes) => {
                let element = self.ctype_from_type_spec(inner);
                ExtVectorLayout::new(&element, *lanes)
                    .map_or_else(|| self.alignof_type_spec(spec), |layout| layout.alignment)
            }
            _ => self.alignof_type_spec(spec),
        }
    }

    /// Convert struct field declarations to StructField for layout computation.
    fn convert_struct_fields(
        &self,
        fields: &[StructFieldDecl],
    ) -> Vec<crate::common::types::StructField> {
        fields
            .iter()
            .filter_map(|f| {
                let ty = self.ctype_from_type_spec_with_derived(&f.type_spec, &f.derived);
                let name = f.name.clone().unwrap_or_default();
                let bit_width = f
                    .bit_width
                    .as_ref()
                    .and_then(|bw| self.eval_const_expr(bw)?.to_i64().map(|v| v as u32));
                let typedef_alignment = |ts: &TypeSpecifier| match ts {
                    TypeSpecifier::TypedefName(td_name) => {
                        self.types.typedef_alignments.get(td_name).copied()
                    }
                    _ => None,
                };
                let alignas_type = f.alignas_type.as_ref().map(|ts| {
                    let natural = self
                        .ctype_from_type_spec(ts)
                        .align_ctx(&*self.types.borrow_struct_layouts());
                    natural.max(typedef_alignment(ts).unwrap_or(0))
                });
                let field_alignment =
                    crate::common::types::strictest_alignment([f.alignment, alignas_type]);
                // A typedef-level `aligned` REPLACES the type's alignment
                // (it may lower it); see `StructField::type_align`.
                let type_align = typedef_alignment(&f.type_spec);
                // C11 6.7.2.1p13: unnamed non-aggregate members are constraint
                // violations GCC ignores entirely; drop from layout + init.
                if f.name.is_none()
                    && bit_width.is_none()
                    && !matches!(
                        ty,
                        crate::common::types::CType::Struct(_)
                            | crate::common::types::CType::Union(_)
                    )
                {
                    return None;
                }
                Some(crate::common::types::StructField {
                    own_cvr: None,
                    name,
                    ty,
                    bit_width,
                    alignment: field_alignment,
                    type_align,
                    is_packed: f.is_packed,
                    base_volatile: f.base_volatile,
                    base_const: crate::common::qualifiers::type_name_const_base(
                        &f.type_spec,
                        f.base_const,
                        &self.type_checker(),
                    ),
                    object_volatile: f.object_volatile,
                    object_const: f.object_const,
                    counted_by: f.counted_by.clone(),
                    pointer_quals: Vec::new(),
                })
            })
            .collect()
    }

    /// Check if an expression is always non-zero at compile time, even if we
    /// can't compute its exact numeric value. String literals are always
    /// non-null pointers, so `"hello" || 0` should evaluate to 1 in static
    /// initializers.
    fn expr_is_always_nonzero(expr: &Expr) -> bool {
        match expr {
            Expr::StringLiteral(..)
            | Expr::WideStringLiteral(..)
            | Expr::Char16StringLiteral(..) => true,
            Expr::Cast(_, inner, _, _) => Self::expr_is_always_nonzero(inner),
            _ => false,
        }
    }

    /// Convert a TypeSpecifier to CType using the TypeContext for typedef/struct resolution.
    /// Convert a TypeSpecifier to CType via `ExprTypeChecker::resolve_type_spec`
    /// so constant evaluation (`_Static_assert`, enum/bitfield bounds, array sizes,
    /// `__builtin_types_compatible_p`) never diverges from expression type checking
    /// or IR lowering on function-pointer parameters, constant array bounds,
    /// nested `typeof(expr)`, address spaces, or inline record definitions.
    fn ctype_from_type_spec(&self, spec: &TypeSpecifier) -> CType {
        self.type_checker().resolve_type_spec(spec)
    }

    /// Convert a TypeSpecifier with derived declarators to CType via the shared
    /// `type_builder` implementation.
    fn ctype_from_type_spec_with_derived(
        &self,
        spec: &TypeSpecifier,
        derived: &[DerivedDeclarator],
    ) -> CType {
        let base = self.ctype_from_type_spec(spec);
        if derived.is_empty() {
            return base;
        }
        crate::common::type_builder::build_full_ctype_with_callbacks(
            base,
            derived,
            &|ts| self.ctype_from_type_spec(ts),
            &|e, _| self.eval_const_expr(e).and_then(|c| c.to_usize()),
        )
    }
}

/// The shadowing decision of [`SemaConstEval::fold_as_library_function`], split
/// out from the symbol table so it can be tested without building a sema
/// context.
///
/// `declared_function` is `None` when nothing visible declares the name,
/// `Some(true)` when the innermost visible declaration is a function, and
/// `Some(false)` when it is anything else (an object, a typedef, an enumerator).
/// `depth` is the scope depth of that declaration, 0 being file scope.
fn library_call_is_foldable(declared_function: Option<bool>, depth: Option<usize>) -> bool {
    match declared_function {
        // Nothing visible: an ordinary call to the library function.
        None => true,
        // A function declaration at file scope is a prototype for the library
        // function itself (or a definition of it, which the A13/A14 symbol
        // inventory handles separately). A block-scope one is either a redundant
        // prototype, where folding is harmless, or a GNU nested function
        // definition, where folding is wrong; distinguishing them needs a scan
        // of the mangled `parent.name` keys in `functions`, which is quadratic,
        // so both are declined -- the safe answer, and the one gcc 14.2 agrees
        // with on every measured cell of the shadowing matrix.
        Some(true) => depth.unwrap_or(0) == 0,
        // An object, typedef or enumerator of that name shadows the library
        // function (C11 6.2.1p4), at any depth.
        Some(false) => false,
    }
}

#[cfg(test)]
mod library_name_shadowing_tests {
    use super::library_call_is_foldable;

    #[test]
    fn nothing_visible_folds_as_the_library_function() {
        assert!(library_call_is_foldable(None, None));
    }

    #[test]
    fn a_file_scope_function_declaration_folds() {
        // The ordinary hosted shape: <string.h> declares `size_t strlen(const
        // char *)` at file scope, and the call is to the library function.
        assert!(library_call_is_foldable(Some(true), Some(0)));
    }

    #[test]
    fn a_block_scope_function_declaration_declines() {
        // Redundant prototype or GNU nested definition: declined, both.
        assert!(!library_call_is_foldable(Some(true), Some(1)));
        assert!(!library_call_is_foldable(Some(true), Some(3)));
    }

    #[test]
    fn an_unknown_depth_is_treated_as_file_scope() {
        // `lookup_depth` answers `None` when the symbol table has no scope for
        // the name (global constant evaluation before any scope is pushed);
        // treating that as file scope keeps static initializers foldable.
        assert!(library_call_is_foldable(Some(true), None));
    }

    #[test]
    fn a_non_function_declaration_declines_at_every_depth() {
        // A pointer object, a plain object, a typedef or an enumerator named
        // after a library function shadows it wherever it is declared.
        assert!(!library_call_is_foldable(Some(false), Some(0)));
        assert!(!library_call_is_foldable(Some(false), Some(1)));
        assert!(!library_call_is_foldable(Some(false), None));
    }

    #[test]
    fn the_decision_space_is_exhaustively_pinned() {
        // Every input the seam can receive, so a future edit cannot flip one
        // case without flipping a pinned answer.
        let table = [
            ((None, None), true),
            ((Some(true), None), true),
            ((Some(true), Some(0)), true),
            ((Some(true), Some(1)), false),
            ((Some(true), Some(2)), false),
            ((Some(false), None), false),
            ((Some(false), Some(0)), false),
            ((Some(false), Some(1)), false),
        ];
        for ((declared, depth), want) in table {
            assert_eq!(
                library_call_is_foldable(declared, depth),
                want,
                "declared={declared:?} depth={depth:?}"
            );
        }
    }
}
