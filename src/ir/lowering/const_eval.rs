//! Constant expression evaluation for the IR lowerer.
//!
//! Core constant-folding logic: integer and floating-point arithmetic, cast
//! chains, sizeof/offsetof evaluation, and the top-level `eval_const_expr`
//! entry point. Global-address resolution lives in `const_eval_global_addr`
//! and initializer-list size computation in `const_eval_init_size`.
//!
//! Shared pure-evaluation logic (literal eval, builtin folding, sub-int promotion,
//! binary operation arithmetic) lives in `common::const_eval` and `common::const_arith`,
//! called by both this module and `sema::const_eval`.

use super::lower::Lowerer;
use crate::common::const_arith;
use crate::common::const_eval as shared_const_eval;
use crate::common::types::{CType, IrType, max_object_bytes};
use crate::frontend::parser::ast::{
    BinOp, DerivedDeclarator, Expr, Initializer, SizeofArg, StructFieldDecl, TypeSpecifier, UnaryOp,
};
use crate::ir::lowering::const_object_images::{self, ConstObjectImage};
use crate::ir::reexports::{GlobalInit, IrConst};

impl Lowerer {
    /// Look up a pre-computed constant value from sema's ConstMap.
    /// Returns Some(IrConst) if sema successfully evaluated this expression
    /// at compile time during its pass.
    pub(super) fn lookup_sema_const(&self, expr: &Expr) -> Option<IrConst> {
        self.sema_const_values.get(&expr.id()).cloned()
    }

    /// Check if an expression tree contains a Sizeof node at any depth.
    /// Used to avoid trusting sema's pre-computed values for expressions involving
    /// sizeof, since sema evaluates sizeof before the lowerer resolves unsized array
    /// dimensions from initializers. For example, `sizeof(cases) / sizeof(cases[0]) + 1`
    /// contains Sizeof nodes in the division subtree, so the entire expression must
    /// be recomputed by the lowerer.
    pub(super) fn expr_contains_sizeof(expr: &Expr) -> bool {
        match expr {
            Expr::Sizeof(_, _) => true,
            Expr::BinaryOp(_, lhs, rhs, _) => {
                Self::expr_contains_sizeof(lhs) || Self::expr_contains_sizeof(rhs)
            }
            Expr::UnaryOp(_, inner, _) | Expr::PostfixOp(_, inner, _) => {
                Self::expr_contains_sizeof(inner)
            }
            Expr::Cast(_, inner, _, _) => Self::expr_contains_sizeof(inner),
            Expr::Conditional(cond, then_e, else_e, _) => {
                Self::expr_contains_sizeof(cond)
                    || Self::expr_contains_sizeof(then_e)
                    || Self::expr_contains_sizeof(else_e)
            }
            Expr::GnuConditional(cond, else_e, _) => {
                Self::expr_contains_sizeof(cond) || Self::expr_contains_sizeof(else_e)
            }
            Expr::Comma(lhs, rhs, _) => {
                Self::expr_contains_sizeof(lhs) || Self::expr_contains_sizeof(rhs)
            }
            _ => false,
        }
    }

    /// Try to evaluate a constant expression at compile time.
    ///
    /// First checks sema's pre-computed ConstMap (O(1) lookup for expressions
    /// that sema could evaluate). Falls back to the lowerer's own evaluation
    /// for expressions that require lowering-specific state (global addresses,
    /// const local values, etc.).
    pub(super) fn eval_const_expr(&self, expr: &Expr) -> Option<IrConst> {
        // Fast path: consult sema's pre-computed constant values.
        // This avoids re-evaluating expressions that sema already handled.
        // We skip the sema lookup for:
        //   - Identifiers: the lowerer may have more information (const local values,
        //     static locals) that sema lacks.
        //   - Expressions containing sizeof: sema may have computed sizeof for unsized
        //     arrays (e.g., `PT cases[] = {1,2,3,...}`) before the lowerer resolved the
        //     actual element count from the initializer. The lowerer's sizeof_expr/
        //     sizeof_type uses the correctly-sized global, so we must recompute.
        //     This check covers sizeof itself and any parent expression containing it,
        //     such as `sizeof(x) / sizeof(x[0]) + 1`.
        let float_cast = matches!(
            expr,
            Expr::Cast(target, _, _, _)
                if matches!(
                    self.type_spec_to_ir(target),
                    IrType::F32 | IrType::F64 | IrType::F128
                )
        );
        // Identifiers are answered first by `eval_const_identifier` (enum
        // constants and const locals, with lowering's own shadowing rules).
        // What it cannot answer may still be a folded const object: sema
        // records an identifier in its const map only when the innermost
        // visible declaration of that name is a const object with a constant
        // initializer (`annotate_const_value` -> `SymbolTable::folded_value`;
        // `const volatile` objects are never marked folded, so a read that
        // must reach memory, C11 6.7.3p7, is never answered here). Without
        // this, a bare `int x = cc;` at file scope fell through to the
        // "initializer element is not constant" diagnostic while `cc * 2`
        // folded through the parent expression's map entry.
        if let Expr::Identifier(name, _) = expr {
            if let Some(val) = self.eval_const_identifier(name) {
                return Some(val);
            }
            return self.lookup_sema_const(expr);
        }
        if !float_cast && !Self::expr_contains_sizeof(expr) {
            if let Some(val) = self.lookup_sema_const(expr) {
                return Some(val);
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
            | Expr::FloatLiteralF128(..) => shared_const_eval::eval_literal(expr),
            // C23 decimal literal: the 16 bytes are the raw BID bit pattern
            // (little-endian), mirroring the lower_expr materialization.
            // Lowering-local (not shared): sema keeps declining decimal
            // constants until its own const users grow BID arms.
            Expr::FloatLiteralDecimal(width, bytes, _) => match *width {
                32 => {
                    let v = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
                    Some(IrConst::D32(v))
                }
                64 => {
                    let v = u64::from_le_bytes([
                        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6],
                        bytes[7],
                    ]);
                    Some(IrConst::D64(v))
                }
                _ => {
                    let v = u128::from_le_bytes(*bytes);
                    Some(IrConst::I128(v as i128))
                }
            },
            Expr::UnaryOp(UnaryOp::Plus, inner, _) => self.eval_const_expr(inner),
            Expr::UnaryOp(UnaryOp::Neg, inner, _) => {
                let val = self.eval_const_expr(inner)?;
                // _Float128 constants are carried as I128 bit patterns
                // (binary128). Float negation flips the SIGN BIT (bit 127),
                // not the integer value: -0x3fff8000... (1.5) must become
                // 0xbfff8000... (-1.5), not 0xc0008000... (-3.0).
                // _Decimal128 rides the same I128 carrier with its sign at
                // bit 127 too. The CType check (not just literal shape)
                // covers const-propagated identifiers; unsigned __int128
                // keeps wrapping negation via the fallthrough below.
                let inner_ct = self.get_expr_ctype(inner);
                if self.infer_expr_type(inner) == IrType::F128
                    || matches!(inner.as_ref(), Expr::FloatLiteralF128(..))
                    || inner_ct == Some(CType::Float128)
                    || inner_ct == Some(CType::Decimal128)
                {
                    if let IrConst::I128(bits) = val {
                        let toggled = (bits as u128) ^ (1u128 << 127);
                        return Some(IrConst::I128(toggled as i128));
                    }
                }
                let promoted =
                    shared_const_eval::promote_sub_int(val, self.is_expr_unsigned_for_const(inner));
                const_arith::negate_const(promoted)
            }
            Expr::BinaryOp(op, lhs, rhs, _) => {
                let l = self.eval_const_expr(lhs);
                let r = self.eval_const_expr(rhs);
                if let (Some(l), Some(r)) = (l, r) {
                    // F33: _Float128/_Decimal128 constants ride the I128
                    // carrier, and the shared integer/float binop folders
                    // would compute on their BITS (`1.5F128 + 1` folded as
                    // integer add). Decline any binop with a 128-bit-float
                    // or decimal side up front (decimal sides already
                    // decline inside via to_i64/to_f64, but one central
                    // guard covers all three widths and both folders).
                    // Logical operators still fold below via is_nonzero,
                    // which is BID-aware for D32/D64.
                    let wide_float_side = |e: &Expr| {
                        matches!(
                            self.get_expr_ctype(e),
                            Some(CType::Float128)
                                | Some(CType::Decimal32)
                                | Some(CType::Decimal64)
                                | Some(CType::Decimal128)
                        )
                    };
                    // A _Float128 side (with no decimal side: mixing is a
                    // constraint violation, C23 6.5.2) folds exactly in
                    // binary128, the common type (C23 6.3.1.8).
                    let (l_ct, r_ct) = (self.get_expr_ctype(lhs), self.get_expr_ctype(rhs));
                    let is_f128 = |ct: &Option<CType>| matches!(ct, Some(CType::Float128));
                    let is_dec = |ct: &Option<CType>| ct.as_ref().is_some_and(CType::is_decimal);
                    if (is_f128(&l_ct) || is_f128(&r_ct)) && !is_dec(&l_ct) && !is_dec(&r_ct) {
                        if let (Some(a), Some(b)) = (
                            shared_const_eval::f128_bytes_of(&l, l_ct.as_ref()),
                            shared_const_eval::f128_bytes_of(&r, r_ct.as_ref()),
                        ) {
                            if let Some(v) = shared_const_eval::fold_f128_binop(op, &a, &b) {
                                return Some(v);
                            }
                        }
                    }
                    if !wide_float_side(lhs) && !wide_float_side(rhs) {
                        // Use infer_expr_type (C semantic types) for proper usual arithmetic
                        // conversions. get_expr_type returns IR storage types (IntLiteral → I64)
                        // which loses 32-bit width info needed for correct folding of
                        // expressions like (1 << 31) / N.
                        let lhs_ty = self.infer_expr_type(lhs);
                        let rhs_ty = self.infer_expr_type(rhs);
                        let result = self.eval_const_binop(op, &l, &r, lhs_ty, rhs_ty);
                        if result.is_some() {
                            return result;
                        }
                    }
                }
                // For LogicalOr/LogicalAnd, handle cases where one operand is a
                // string literal or other always-nonzero expression (e.g., address-of).
                // String literals are always non-null pointers, so:
                //   "str" || x  =>  1
                //   x || "str"  =>  1
                //   "str" && x  =>  bool(x) if x is evaluable, else 1 if x is also nonzero
                //   0 && "str"  =>  0
                if *op == BinOp::LogicalOr {
                    // `||` evaluates the LHS, then the RHS only if the LHS is
                    // false. Folding to a constant is sound ONLY when the
                    // skipped side cannot have effects that must be observed:
                    //   - LHS always-nonzero: RHS is skipped -> 1 (sound even if
                    //     RHS has side effects, because short-circuit skips it).
                    //   - RHS always-nonzero/known but LHS UNKNOWN: the LHS must
                    //     STILL be evaluated for its side effects (e.g.
                    //     `(*q = 10) || 60` stores to *q before 60 is considered).
                    //     Folding to 1 drops that store -> UNSOUND. Only fold via
                    //     the RHS when the LHS is a known (pure) constant.
                    let l_nonzero = l.as_ref().is_some_and(|v| self.const_truth(v, lhs))
                        || Self::expr_is_always_nonzero(lhs);
                    let r_nonzero = r.as_ref().is_some_and(|v| self.const_truth(v, rhs))
                        || Self::expr_is_always_nonzero(rhs);
                    if l_nonzero || (r_nonzero && l.is_some()) {
                        return Some(IrConst::I64(1));
                    }
                    // Both are zero constants => result is 0 (both pure).
                    if l.as_ref().is_some_and(|v| !self.const_truth(v, lhs))
                        && r.as_ref().is_some_and(|v| !self.const_truth(v, rhs))
                    {
                        return Some(IrConst::I64(0));
                    }
                }
                if *op == BinOp::LogicalAnd {
                    // `&&` evaluates the LHS, then the RHS only if the LHS is
                    // true. Folding to 0 is sound when the skipped side cannot
                    // have unobservable-missing effects:
                    //   - LHS known-zero: RHS is skipped -> 0 (sound).
                    //   - RHS known-zero but LHS UNKNOWN: the LHS must still be
                    //     evaluated for side effects -> UNSOUND to fold to 0.
                    //     Only fold via the RHS when the LHS is a known constant.
                    if l.as_ref().is_some_and(|v| !self.const_truth(v, lhs))
                        || (r.as_ref().is_some_and(|v| !self.const_truth(v, rhs)) && l.is_some())
                    {
                        return Some(IrConst::I64(0));
                    }
                    // Both sides known nonzero (constants or always-nonzero pure
                    // expressions such as string literals): result is 1. Both
                    // operands are side-effect-free here, so folding is sound.
                    let l_nonzero = l.as_ref().is_some_and(|v| self.const_truth(v, lhs))
                        || Self::expr_is_always_nonzero(lhs);
                    let r_nonzero = r.as_ref().is_some_and(|v| self.const_truth(v, rhs))
                        || Self::expr_is_always_nonzero(rhs);
                    if l_nonzero && r_nonzero {
                        return Some(IrConst::I64(1));
                    }
                }
                // For subtraction, try evaluating as pointer difference:
                // &arr[5] - &arr[0], (char*)&s.c - (char*)&s.a, etc.
                // Both operands may be global address expressions referring to the
                // same symbol; the result is the byte offset difference (possibly
                // divided by the pointed-to element size for typed pointer subtraction).
                if *op == BinOp::Sub {
                    return self.eval_const_ptr_diff(lhs, rhs);
                }
                None
            }
            Expr::UnaryOp(UnaryOp::BitNot, inner, _) => {
                let val = self.eval_const_expr(inner)?;
                let is_unsigned = self.is_expr_unsigned_for_const(inner);
                let promoted = shared_const_eval::promote_sub_int(val, is_unsigned);
                let result = const_arith::bitnot_const(promoted)?;
                // C11 6.5.3.3: the result of `~` has the promoted type of the
                // operand. bitnot_const complements the FULL stored width, which
                // over-widens when a 32-bit unsigned int is stored as I64:
                // `~0xFFFFFFFFu` complements to 0xFFFFFFFF_00000000 instead of 0,
                // flipping a boolean/short-circuit use (bug found via Regehr
                // yarpgen test regress_logical_and_const_bitnot_short_circuit:
                // `const unsigned u = 0xFFFFFFFFu; ~u && f()` wrongly called f()).
                // Truncate to the operand's promoted width (integer promotion
                // lifts sub-int to at least int, i.e. 4 bytes).
                let inner_ty = self.infer_expr_type(inner);
                if inner_ty.size() < 8 {
                    if let IrConst::I64(v) = result {
                        return Some(IrConst::I64((v as u32) as i64));
                    }
                }
                Some(result)
            }
            Expr::Cast(target_type, inner, _, _) => self.eval_const_cast(target_type, inner),
            Expr::Identifier(name, _) => self.eval_const_identifier(name),
            Expr::Sizeof(arg, _) => {
                // VLA sizeof is a runtime value (C99 6.7.6.2 / 6.5.3.4): the
                // size is captured at the point of the declaration, not a
                // compile-time constant. Folding it via sizeof_type() yields
                // the pointer-size fallback for Array(_, None) (e.g. 8*3=24
                // instead of (i+2)*sizeof(int)*3). GCC torture 20040411-1,
                // 20040423-1, 20041218-2.
                if self.sizeof_arg_is_vla(arg) {
                    return None;
                }
                let size = match arg.as_ref() {
                    SizeofArg::Type(ts) => self.sizeof_type(ts),
                    SizeofArg::Expr(e) => self.sizeof_expr(e),
                };
                Some(IrConst::I64(size as i64))
            }
            Expr::Alignof(ts, _) => {
                let align = self.c11_alignof_type(ts);
                Some(IrConst::I64(align as i64))
            }
            Expr::AlignofExpr(inner_expr, _) => {
                let align = self.alignof_expr(inner_expr);
                Some(IrConst::I64(align as i64))
            }
            Expr::GnuAlignof(ts, _) => {
                let align = self.preferred_alignof_type(ts);
                Some(IrConst::I64(align as i64))
            }
            Expr::GnuAlignofExpr(inner_expr, _) => {
                let align = self.preferred_alignof_expr(inner_expr);
                Some(IrConst::I64(align as i64))
            }
            Expr::Conditional(cond, then_e, else_e, _) => {
                // Ternary in constant expr: evaluate condition and pick branch
                let cond_val = self.eval_const_expr(cond)?;
                if self.const_truth(&cond_val, cond) {
                    self.eval_const_expr(then_e)
                } else {
                    self.eval_const_expr(else_e)
                }
            }
            Expr::GnuConditional(cond, else_e, _) => {
                let cond_val = self.eval_const_expr(cond)?;
                if self.const_truth(&cond_val, cond) {
                    Some(cond_val) // condition value is used as result
                } else {
                    self.eval_const_expr(else_e)
                }
            }
            Expr::UnaryOp(UnaryOp::LogicalNot, inner, _) => {
                if let Some(val) = self.eval_const_expr(inner) {
                    Some(IrConst::I64(if self.const_truth(&val, inner) {
                        0
                    } else {
                        1
                    }))
                } else if Self::expr_is_always_nonzero(inner) {
                    // !("string_literal") is always 0
                    Some(IrConst::I64(0))
                } else {
                    None
                }
            }
            // Handle &((type*)0)->member pattern (offsetof)
            Expr::AddressOf(inner, _) => self.eval_offsetof_pattern(inner),
            Expr::BuiltinTypesCompatibleP(type1, type2, qualifiers1, qualifiers2, _) => {
                let result = self.eval_types_compatible(type1, type2, qualifiers1, qualifiers2);
                Some(IrConst::I64(result as i64))
            }
            // Handle compile-time builtin function calls in constant expressions.
            Expr::FunctionCall(func, args, _) => {
                if let Expr::Identifier(name, _) = func.as_ref() {
                    let folded = shared_const_eval::eval_builtin_call_with_mode(
                        name.as_str(),
                        args,
                        &|e| self.eval_const_expr(e),
                        self.builtin_policy.may_assume(name.as_str()),
                    );
                    if folded.is_none()
                        && name == "__builtin_constant_p"
                        && self.folding_initializer.get()
                    {
                        return Some(IrConst::I32(0));
                    }
                    folded
                } else {
                    None
                }
            }
            Expr::CompoundLiteral(type_spec, init, _) => {
                self.eval_const_compound_literal(type_spec, init)
            }
            Expr::GenericSelection(controlling, associations, _) => {
                let selected = self.resolve_generic_selection_expr(controlling, associations)?;
                self.eval_const_expr(selected)
            }
            // A read of an element or member of a global object whose
            // initializer is a constant byte image: `table[3]`,
            // `config.limit`, `matrix[1][2]`.  sema decides whether such a
            // read is a constant (`static_init.rs::folded_object_read`) and
            // rejects the program otherwise; the value here comes from the
            // bytes the initializer emits, so it cannot disagree with what the
            // program loads.
            Expr::ArraySubscript(..) | Expr::MemberAccess(..) => self.eval_const_object_read(expr),
            _ => None,
        }
    }

    /// Evaluate a cast expression at compile time.
    /// The C truth value of a folded constant, decided by `expr`'s C type
    /// (see `common::const_eval::const_truth`).
    pub(super) fn const_truth(&self, val: &IrConst, expr: &Expr) -> bool {
        shared_const_eval::const_truth(val, self.get_expr_ctype(expr).as_ref())
    }

    pub(super) fn eval_const_cast(
        &self,
        target_type: &TypeSpecifier,
        inner: &Expr,
    ) -> Option<IrConst> {
        let target_ir_ty = self.type_spec_to_ir(target_type);

        // _Float128 (IEEE binary128, carried as IrType::U128) is never an
        // integer or f64 bit-cast: the 16 bytes ARE the binary128 payload.
        // Both directions convert by value through the shared exact helpers
        // (`(double)1.5Q` is 1.5, `(_Float128)3` is binary128 3.0), which
        // round exactly like libgcc's __trunctfdf2/__extenddftf2 & co.
        let target_ct = self.type_spec_to_ctype(target_type);
        let src_ct = self.get_expr_ctype(inner);
        let src_val = self.eval_const_expr(inner)?;

        // C11 6.3.1.2p1: conversion to _Bool yields 0 or 1.  The constant
        // cast must NORMALIZE, not bit-copy: `(_Bool)5` is 1, not 5 (the
        // runtime path emits `val != 0`; the fold must mirror it, including
        // NaN -> 1, ±0.0 -> 0, negative integers -> 1).  Decimal constants
        // fold through BID-decoded truthiness (+0 == -0), exactly what the
        // runtime lower_soft_float_truthiness computes.
        if target_ct == CType::Bool {
            // The C type decides: a long double's value (not its f64
            // shadow: 1e-400L is true), and the I128 carrier of -0.0Q or
            // -0.0DL is false.
            let truth = shared_const_eval::const_truth(&src_val, src_ct.as_ref());
            return Some(IrConst::I8(i8::from(truth)));
        }

        // Decimal-target casts fold through the BID master conversion, so
        // `(_Decimal32)5` and `(_Decimal64)1.5` work in static initializers
        // exactly like their literal forms. Unknown source kinds decline
        // (the I128 carrier is ambiguous: never guess).
        if target_ct.is_decimal() {
            let width = match target_ct {
                CType::Decimal32 => 32,
                CType::Decimal64 => 64,
                _ => 128,
            };
            if let Some(ref src_ct) = src_ct {
                if let Some(bid) = const_arith::const_to_bid(&src_val, src_ct, width) {
                    return Some(bid);
                }
            }
            return None;
        }

        // _Float128 target: the exact binary128 value of the source.
        if target_ct == CType::Float128 {
            let bytes = shared_const_eval::f128_bytes_of(&src_val, src_ct.as_ref())?;
            return Some(IrConst::I128(u128::from_le_bytes(bytes) as i128));
        }
        // _Float128 or long double source: convert its binary128 value (a long
        // double's in the target format, never its f64 shadow). NaN and
        // infinity to an integer stay unfolded, as for double (IEEE invalid
        // at run time); a static initializer then takes sema's GCC-style
        // fold.
        let f128_src = match (&src_ct, &src_val) {
            (Some(CType::Float128), _) => {
                shared_const_eval::f128_bytes_of(&src_val, src_ct.as_ref())
            }
            (_, IrConst::LongDouble(_, bytes)) => {
                Some(crate::common::long_double::long_double_value_bytes(bytes))
            }
            _ => None,
        };
        if let Some(bytes) = f128_src {
            let nonfinite = (u128::from_le_bytes(bytes) >> 112) & 0x7FFF == 0x7FFF;
            if nonfinite && target_ct.is_integer() {
                return None;
            }
            return match shared_const_eval::f128_to_ctype(&bytes, &target_ct)? {
                c @ (IrConst::F32(_)
                | IrConst::F64(_)
                | IrConst::LongDouble(..)
                | IrConst::I128(_)) => Some(c),
                c => {
                    let bits = c.to_i64()? as u64;
                    self.cast_bits_to_ir_const(bits, !target_ct.is_unsigned(), target_ir_ty)
                }
            };
        }
        if let Some(fv) = src_val.to_f64() {
            if matches!(&src_val, IrConst::F32(_) | IrConst::F64(_)) {
                return IrConst::cast_float_to_target(fv, target_ir_ty);
            }
        }

        // Decimal sources convert by value (decode + truncate/re-encode),
        // never by carrier bits: `(int)2.5DF` is 2, `(double)1.5DD` is 1.5.
        // _Decimal128 rides I128, so it needs the CType check; D32/D64 are
        // unambiguous variants.
        if src_ct == Some(CType::Decimal128) && matches!(src_val, IrConst::I128(_)) {
            return Self::cast_bid_to_ir_type(&src_val, target_ir_ty, &target_ct);
        }
        if matches!(src_val, IrConst::D32(_) | IrConst::D64(_)) {
            return Self::cast_bid_to_ir_type(&src_val, target_ir_ty, &target_ct);
        }

        // Handle I128 source: use full 128-bit value to avoid truncation
        if let IrConst::I128(v128) = src_val {
            let src_ty = self.get_expr_type(inner);
            let src_unsigned = src_ty.is_unsigned();
            return Some(Self::cast_i128_to_ir_type(v128, target_ir_ty, src_unsigned));
        }

        // Integer source: use bit-based value extraction, but take signedness
        // from the C expression type. IrConst::I64 stores both signed and
        // unsigned 64-bit bit patterns; `~0ULL` is represented as I64(-1) and
        // must still convert to float as UINT64_MAX, not -1.
        //
        // Prefer the evaluated value's own bits: walking the expression
        // descends into a float leaf for float->int cast chains and calls
        // irconst_to_bits on a float constant there, which falls back to 0
        // (`(uint32_t)(int)3.999L` folded to 0 instead of 3). For pure
        // integer chains both sources agree; the value form also survives
        // sign-extending narrow casts (`(i8)255` -> 0xffffffff).
        let (bits, _) = match src_val.to_i64() {
            Some(v) => (v as u64, true),
            None => self.eval_const_expr_as_bits(inner)?,
        };
        let src_signed = !self.is_expr_unsigned_for_const(inner);

        // For 128-bit targets, sign/zero-extend based on source signedness
        if matches!(target_ir_ty, IrType::I128 | IrType::U128) {
            let src_ty = self.get_expr_type(inner);
            let v128 = if src_ty.is_unsigned() {
                bits as i128
            } else {
                (bits as i64) as i128
            };
            return Some(IrConst::I128(v128));
        }

        self.cast_bits_to_ir_const(bits, src_signed, target_ir_ty)
    }

    /// Convert raw bits to an IrConst of the given target type, truncating as needed.
    fn cast_bits_to_ir_const(
        &self,
        bits: u64,
        src_signed: bool,
        target_ir_ty: IrType,
    ) -> Option<IrConst> {
        let target_width = target_ir_ty.size() * 8;
        let truncated = if target_width >= 64 {
            bits
        } else {
            bits & ((1u64 << target_width) - 1)
        };
        let result = match target_ir_ty {
            IrType::I8 => IrConst::I8(truncated as i8),
            IrType::U8 => IrConst::I64(truncated as u8 as i64),
            IrType::I16 => IrConst::I16(truncated as i16),
            IrType::U16 => IrConst::I64(truncated as u16 as i64),
            IrType::I32 => IrConst::I32(truncated as i32),
            IrType::U32 => IrConst::I64(truncated as u32 as i64),
            IrType::I64 | IrType::U64 => IrConst::I64(truncated as i64),
            IrType::Ptr => IrConst::ptr_int(truncated as i64),
            IrType::I128 | IrType::U128 => unreachable!("handled above"),
            IrType::F32 => {
                let int_val = if src_signed {
                    bits as i64 as f32
                } else {
                    bits as f32
                };
                IrConst::F32(int_val)
            }
            IrType::F64 => {
                let int_val = if src_signed {
                    bits as i64 as f64
                } else {
                    bits as f64
                };
                IrConst::F64(int_val)
            }
            _ => return None,
        };
        Some(result)
    }

    /// Convert a decimal constant (D32/D64/I128-carried D128) to an integer
    /// or binary-float constant of `target_ir_ty` under C cast semantics.
    /// Integers truncate toward zero with range checks; floats parse from
    /// the exact decimal expansion (correctly rounded, like strtod).
    /// Decimal targets never reach here (folded earlier); ambiguous U128
    /// targets resolve through `target_ct`. None on overflow/UB (decline,
    /// like GCC errors) — never carrier-bit garbage.
    fn cast_bid_to_ir_type(
        val: &IrConst,
        target_ir_ty: IrType,
        target_ct: &CType,
    ) -> Option<IrConst> {
        use crate::common::decimal::*;
        #[derive(Clone, Copy)]
        enum Dec {
            Finite(bool, u128, i32),
            Inf(bool),
            Nan(bool),
        }
        let dec = match *val {
            IrConst::D32(v) => match decode_bid32(v) {
                Some((neg, c, e)) => Dec::Finite(neg, c as u128, e),
                None => {
                    let (neg, is_nan) = bid32_special(v);
                    if is_nan { Dec::Nan(neg) } else { Dec::Inf(neg) }
                }
            },
            IrConst::D64(v) => match decode_bid64(v) {
                Some((neg, c, e)) => Dec::Finite(neg, c as u128, e),
                None => {
                    let (neg, is_nan) = bid64_special(v);
                    if is_nan { Dec::Nan(neg) } else { Dec::Inf(neg) }
                }
            },
            IrConst::I128(v) => {
                let b = v as u128;
                let hi = (b >> 64) as u64;
                match decode_bid128(hi, b as u64) {
                    Some((neg, c, e)) => Dec::Finite(neg, c, e),
                    None => {
                        let (neg, is_nan) = bid128_special_hi(hi);
                        if is_nan { Dec::Nan(neg) } else { Dec::Inf(neg) }
                    }
                }
            }
            _ => return None,
        };
        // Correctly-rounded decimal -> binary-float (strtod semantics).
        let to_f64 = || -> Option<f64> {
            match dec {
                Dec::Finite(neg, c, e) => format!("{}{c}e{e}", if neg { "-" } else { "" })
                    .parse::<f64>()
                    .ok(),
                Dec::Inf(neg) => Some(if neg {
                    f64::NEG_INFINITY
                } else {
                    f64::INFINITY
                }),
                Dec::Nan(neg) => Some(if neg { -f64::NAN } else { f64::NAN }),
            }
        };
        match target_ir_ty {
            IrType::F32 => {
                let f = match dec {
                    Dec::Finite(neg, c, e) => format!("{}{c}e{e}", if neg { "-" } else { "" })
                        .parse::<f32>()
                        .ok()?,
                    Dec::Inf(neg) => {
                        if neg {
                            f32::NEG_INFINITY
                        } else {
                            f32::INFINITY
                        }
                    }
                    Dec::Nan(neg) => {
                        if neg {
                            -f32::NAN
                        } else {
                            f32::NAN
                        }
                    }
                };
                Some(IrConst::F32(f))
            }
            IrType::F64 => Some(IrConst::F64(to_f64()?)),
            IrType::F128 => Some(IrConst::long_double(to_f64()?)),
            IrType::I8
            | IrType::U8
            | IrType::I16
            | IrType::U16
            | IrType::I32
            | IrType::U32
            | IrType::I64
            | IrType::U64
            | IrType::I128
            | IrType::U128 => {
                let Dec::Finite(neg, c, e) = dec else {
                    // NaN/Inf -> integer is UB: decline.
                    return None;
                };
                // 128-bit carriers resolve through the target CType so a
                // decimal->128-bit-int cast never emits BID/F128 bits.
                if matches!(target_ir_ty, IrType::I128 | IrType::U128)
                    && !matches!(target_ct, CType::Int128 | CType::UInt128)
                {
                    return None;
                }
                let bits = target_ir_ty.size() as u32 * 8;
                let v = bid_to_int(c, e, neg, bits, target_ir_ty.is_unsigned())?;
                Some(match target_ir_ty {
                    IrType::I8 => IrConst::I8(v as i8),
                    IrType::U8 => IrConst::I64(v as u8 as i64),
                    IrType::I16 => IrConst::I16(v as i16),
                    IrType::U16 => IrConst::I64(v as u16 as i64),
                    IrType::I32 => IrConst::I32(v as i32),
                    IrType::U32 => IrConst::I64(v as u32 as i64),
                    IrType::I64 | IrType::U64 => IrConst::I64(v as i64),
                    IrType::I128 | IrType::U128 => IrConst::I128(v),
                    _ => return None,
                })
            }
            _ => None,
        }
    }

    fn eval_const_identifier(&self, name: &str) -> Option<IrConst> {
        let is_local = self
            .func_state
            .as_ref()
            .is_some_and(|fs| fs.locals.contains_key(name));
        if !is_local {
            if let Some(constant) = self.types.enum_constants.get(name) {
                return Some(IrConst::I64(constant.value));
            }
        }
        if let Some(ref fs) = self.func_state {
            if let Some(&val) = fs.const_local_values.get(name) {
                return Some(IrConst::I64(val));
            }
        }
        None
    }

    /// Evaluate scalar compound literals in constant expressions.
    /// Returns None for multi-field aggregates (those go through struct init path).
    fn eval_const_compound_literal(
        &self,
        type_spec: &TypeSpecifier,
        init: &Initializer,
    ) -> Option<IrConst> {
        let cl_ctype = self.type_spec_to_ctype(type_spec);
        let is_multi_field_aggregate = match &cl_ctype {
            CType::Struct(key) | CType::Union(key) => {
                if let Some(layout) = self.types.borrow_struct_layouts().get(&**key) {
                    layout.fields.len() > 1
                        || layout.fields.iter().any(|f| {
                            matches!(
                                f.ty,
                                CType::Array(..) | CType::Struct(..) | CType::Union(..)
                            )
                        })
                } else {
                    false
                }
            }
            CType::Array(..) => true,
            _ => false,
        };
        if is_multi_field_aggregate {
            None
        } else {
            self.eval_const_initializer_scalar(init)
        }
    }

    /// Fold a read of an element or member of a constant global object.
    ///
    /// The selectors are peeled off outermost-first, then applied
    /// innermost-first against the object's declared type, accumulating a byte
    /// offset into the object's image; the leaf is decoded from the image at
    /// that offset -- the very bytes the program will load.
    fn eval_const_object_read(&self, expr: &Expr) -> Option<IrConst> {
        let mut selectors = Vec::new();
        let mut cursor = expr;
        loop {
            match cursor {
                Expr::ArraySubscript(base, _, _) | Expr::MemberAccess(base, _, _) => {
                    selectors.push(cursor);
                    cursor = base;
                }
                Expr::Identifier(name, _) => {
                    let image = self.const_object_images.get(name.as_str())?;
                    return self.read_const_object(image, &selectors);
                }
                _ => return None,
            }
        }
    }

    /// Apply `selectors` (outermost first) to a constant object's image.
    fn read_const_object(&self, image: &ConstObjectImage, selectors: &[&Expr]) -> Option<IrConst> {
        let layouts = self.types.borrow_struct_layouts();
        let mut ctype = image.ctype.clone();
        let mut offset = 0usize;
        for selector in selectors.iter().rev() {
            match selector {
                Expr::ArraySubscript(_, index, _) => {
                    let element = match &ctype {
                        CType::Array(element, _) => (**element).clone(),
                        _ => return None,
                    };
                    let stride = self.resolve_ctype_size(&element);
                    if stride == 0 {
                        return None;
                    }
                    let index = self.eval_const_expr(index)?.to_i64()?;
                    if index < 0 {
                        return None;
                    }
                    offset = offset.checked_add((index as usize).checked_mul(stride)?)?;
                    ctype = element;
                }
                Expr::MemberAccess(_, field_name, _) => {
                    let key = match &ctype {
                        CType::Struct(key) | CType::Union(key) => key.clone(),
                        _ => return None,
                    };
                    let layout = layouts.get(&*key)?;
                    // A volatile-qualified MEMBER keeps its access even when
                    // the containing object is const (C11 6.7.3p7): `struct
                    // cs { const volatile int f; }; const struct cs csv` must
                    // load `csv.f` every time.  GCC folds this shape; lccc
                    // deliberately does not (check_const_volatile_objects.sh
                    // pins the divergence), so the image fold declines here.
                    if layout
                        .fields
                        .iter()
                        .find(|f| f.name == *field_name)
                        .is_some_and(|f| f.base_volatile || f.object_volatile)
                    {
                        return None;
                    }
                    let (field_offset, field_ty, bit_offset, bit_width) =
                        layout.field_offset_with_bitfield(field_name, &*layouts)?;
                    offset = offset.checked_add(field_offset)?;
                    if let (Some(bit_offset), Some(bit_width)) = (bit_offset, bit_width) {
                        // A bitfield leaf: its storage unit is the declared
                        // type, and the value is the field's bits, signed by
                        // the *field's* type (`int a : 4` of all ones is -1,
                        // `unsigned a : 4` is 15).
                        let unit = self.resolve_ctype_size(&field_ty);
                        let raw = const_object_images::read_le(&image.bytes, offset, unit)?;
                        return const_object_images::extract_bitfield(
                            raw,
                            bit_offset,
                            bit_width,
                            !field_ty.is_unsigned(),
                        );
                    }
                    ctype = field_ty;
                }
                _ => return None,
            }
        }
        // The whole leaf has to be inside the image: an offset that reaches
        // past it means the layout and the initializer disagree, and a fold
        // would invent a value.
        let width = self.resolve_ctype_size(&ctype);
        if offset.checked_add(width)? > image.bytes.len() {
            return None;
        }
        self.read_const_scalar(&image.bytes, offset, &ctype)
    }

    /// Decode one scalar of `ctype` out of an object's byte image.
    fn read_const_scalar(&self, bytes: &[u8], offset: usize, ctype: &CType) -> Option<IrConst> {
        let width = self.resolve_ctype_size(ctype);
        let raw = const_object_images::read_le(bytes, offset, width)?;
        match ctype {
            CType::Float => Some(IrConst::F32(f32::from_bits(raw as u32))),
            CType::Double => Some(IrConst::F64(f64::from_bits(raw))),
            _ if ctype.is_integer() => match width {
                1 => Some(IrConst::I8(raw as i8)),
                2 => Some(IrConst::I16(raw as i16)),
                4 => Some(IrConst::I32(raw as i32)),
                8 => Some(IrConst::I64(raw as i64)),
                _ => None,
            },
            _ => None,
        }
    }

    /// Evaluate an initializer to a scalar constant for use in constant expressions.
    /// Handles both direct expressions and brace-wrapped lists (including nested ones
    /// like `{ { 42 } }` which occur when a struct compound literal has a single field).
    fn eval_const_initializer_scalar(&self, init: &Initializer) -> Option<IrConst> {
        match init {
            Initializer::Expr(expr) => self.eval_const_expr(expr),
            Initializer::List(items) => {
                // For a struct/union compound literal with one field,
                // the initializer list has one item whose value is the scalar.
                // Recurse into the first item.
                if let Some(first) = items.first() {
                    self.eval_const_initializer_scalar(&first.init)
                } else {
                    None
                }
            }
        }
    }

    /// Evaluate the offsetof pattern: &((type*)0)->member
    /// Also handles nested member access like &((type*)0)->data.x
    /// Returns Some(IrConst with offset) if the expression matches the pattern.
    pub(super) fn eval_offsetof_pattern(&self, expr: &Expr) -> Option<IrConst> {
        let (offset, _ty) = self.eval_offsetof_pattern_with_type(expr)?;
        Some(IrConst::ptr_int(offset as i64))
    }

    /// Evaluate an offsetof sub-expression, returning both the accumulated byte offset
    /// and the CType of the resulting expression (needed for chained member access).
    fn eval_offsetof_pattern_with_type(&self, expr: &Expr) -> Option<(usize, CType)> {
        match expr {
            Expr::PointerMemberAccess(base, field_name, _) => {
                // base should be (type*)0 - a cast of 0 to a pointer type
                let (type_spec, base_offset) = self.extract_null_pointer_cast_with_offset(base)?;
                let layout = self.get_struct_layout_for_type(&type_spec)?;
                let (field_offset, field_ty) =
                    layout.field_offset(field_name, &*self.types.borrow_struct_layouts())?;
                Some((base_offset + field_offset, field_ty))
            }
            Expr::MemberAccess(base, field_name, _) => {
                // First try: base is *((type*)0) (deref pattern)
                if let Expr::Deref(inner, _) = base.as_ref() {
                    let (type_spec, base_offset) =
                        self.extract_null_pointer_cast_with_offset(inner)?;
                    let layout = self.get_struct_layout_for_type(&type_spec)?;
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
                // Handle &((type*)0)->member[index] pattern
                let (base_offset, base_type) = self.eval_offsetof_pattern_with_type(base)?;
                let idx_val = self.eval_const_expr(index)?;
                let idx = idx_val.to_i64()?;
                let elem_size = match &base_type {
                    CType::Array(elem, _) => self.resolve_ctype_size(elem),
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

    /// Extract the struct type from a (type*)0 pattern, returning the base TypeSpecifier
    /// for the struct type and any accumulated offset from nested member access.
    fn extract_null_pointer_cast_with_offset(&self, expr: &Expr) -> Option<(TypeSpecifier, usize)> {
        match expr {
            Expr::Cast(type_spec, inner, _, _) => {
                // The type should be a Pointer to a struct
                if let TypeSpecifier::Pointer(inner_ts, _) = type_spec {
                    // Check that the inner expression is 0
                    if self.is_zero_expr(inner) {
                        return Some((*inner_ts.clone(), 0));
                    }
                }
                None
            }
            _ => None,
        }
    }

    /// Check if an expression evaluates to 0 (integer literal 0 or cast of 0).
    fn is_zero_expr(&self, expr: &Expr) -> bool {
        const_arith::is_zero_expr(expr)
    }

    /// Evaluate a constant expression, returning raw u64 bits and signedness.
    /// This preserves signedness information through cast chains.
    /// Signedness determines how the value is widened in the next cast.
    fn eval_const_expr_as_bits(&self, expr: &Expr) -> Option<(u64, bool)> {
        match expr {
            Expr::Cast(target_type, inner, _, _) => {
                let (bits, _src_signed) = self.eval_const_expr_as_bits(inner)?;
                let target_ir_ty = self.type_spec_to_ir(target_type);
                let target_width = target_ir_ty.size() * 8;
                let target_signed = matches!(
                    target_ir_ty,
                    IrType::I8 | IrType::I16 | IrType::I32 | IrType::I64
                );
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

    /// Cast a full 128-bit integer value to an IrType without going through u64 truncation.
    ///
    /// For targets <= 64 bits, extracts the lower bits. For 128-bit targets, preserves the
    /// full value. For float targets, uses value-based conversion from the full i128.
    fn cast_i128_to_ir_type(v128: i128, target: IrType, src_unsigned: bool) -> IrConst {
        let bits_lo = v128 as u64; // lower 64 bits
        match target {
            IrType::I8 => IrConst::I8(bits_lo as i8),
            IrType::U8 => IrConst::I64(bits_lo as u8 as i64),
            IrType::I16 => IrConst::I16(bits_lo as i16),
            IrType::U16 => IrConst::I64(bits_lo as u16 as i64),
            IrType::I32 => IrConst::I32(bits_lo as i32),
            IrType::U32 => IrConst::I64(bits_lo as u32 as i64),
            IrType::I64 | IrType::U64 => IrConst::I64(bits_lo as i64),
            IrType::Ptr => IrConst::ptr_int(bits_lo as i64),
            IrType::I128 | IrType::U128 => IrConst::I128(v128),
            IrType::F32 => {
                // int-to-float: signedness comes from the source type
                let fv = if src_unsigned {
                    (v128 as u128) as f32
                } else {
                    v128 as f32
                };
                IrConst::F32(fv)
            }
            IrType::F64 => {
                let fv = if src_unsigned {
                    (v128 as u128) as f64
                } else {
                    v128 as f64
                };
                IrConst::F64(fv)
            }
            IrType::F128 => {
                // Use direct integer-to-x87 conversion to preserve full 64-bit
                // mantissa precision (x87 has 64-bit mantissa, unlike f64's 52-bit).
                if src_unsigned {
                    IrConst::long_double_from_u128(v128 as u128)
                } else {
                    IrConst::long_double_from_i128(v128)
                }
            }
            _ => IrConst::I128(v128), // fallback: preserve value
        }
    }

    /// Evaluate a constant binary operation.
    /// Delegates to `shared_const_eval::eval_binop_with_types` which implements
    /// C's usual arithmetic conversions (C11 6.3.1.8).
    fn eval_const_binop(
        &self,
        op: &BinOp,
        lhs: &IrConst,
        rhs: &IrConst,
        lhs_ty: IrType,
        rhs_ty: IrType,
    ) -> Option<IrConst> {
        // Raw (unpromoted) sizes: the shared helper applies C11 6.3.1.1
        // integer promotion, which turns a sub-int unsigned operand into a
        // SIGNED int. Clamping here with `.max(4)` while keeping the
        // unpromoted signedness would fold `(uint8_t)x | (int16_t)y` as
        // unsigned.
        shared_const_eval::eval_binop_with_types(
            op,
            lhs,
            rhs,
            lhs_ty.size(),
            lhs_ty.is_unsigned(),
            rhs_ty.size(),
            rhs_ty.is_unsigned(),
        )
    }

    /// Check if an expression has an unsigned type for constant evaluation.
    pub(super) fn is_expr_unsigned_for_const(&self, expr: &Expr) -> bool {
        if let Expr::Cast(target_type, _, _, _) = expr {
            let ty = self.type_spec_to_ir(target_type);
            return ty.is_unsigned();
        }
        let ty = self.infer_expr_type(expr);
        ty.is_unsigned()
    }

    /// Try to constant-fold a binary operation from its parts.
    /// Used by lower_binary_op to avoid generating IR for constant expressions,
    /// ensuring correct C type semantics (especially 32-bit vs 64-bit width).
    pub(super) fn eval_const_expr_from_parts(
        &self,
        op: &BinOp,
        lhs: &Expr,
        rhs: &Expr,
    ) -> Option<IrConst> {
        let l = self.eval_const_expr(lhs)?;
        let r = self.eval_const_expr(rhs)?;
        // F33: decline 128-bit-float/decimal sides (same guard as the
        // BinaryOp arm above: the shared folders would compute on
        // carrier BITS).
        for e in [lhs, rhs] {
            if matches!(
                self.get_expr_ctype(e),
                Some(CType::Float128)
                    | Some(CType::Decimal32)
                    | Some(CType::Decimal64)
                    | Some(CType::Decimal128)
            ) {
                return None;
            }
        }
        let lhs_ty = self.infer_expr_type(lhs);
        let rhs_ty = self.infer_expr_type(rhs);
        let result = self.eval_const_binop(op, &l, &r, lhs_ty, rhs_ty)?;
        // Convert to I64 for IR operand compatibility (IR uses I64 for all int operations).
        Some(match result {
            IrConst::I32(v) => IrConst::I64(v as i64),
            IrConst::I8(v) => IrConst::I64(v as i64),
            IrConst::I16(v) => IrConst::I64(v as i64),
            other => other,
        })
    }

    /// Evaluate a constant expression and return as usize (for array index designators).
    pub(super) fn eval_const_expr_for_designator(&self, expr: &Expr) -> Option<usize> {
        self.eval_const_expr(expr).and_then(|v| v.to_usize())
    }

    /// Evaluate a compile-time pointer difference expression.
    /// Handles patterns like:
    ///   &arr[5] - &arr[0]              -> 5  (typed pointer subtraction)
    ///   (char*)&arr[5] - (char*)&arr[0] -> 20 (byte-level subtraction)
    ///   (long)&arr[5] - (long)&arr[0]  -> 20 (cast-to-integer subtraction)
    ///   (char*)&s.c - (char*)&s.a      -> 8  (struct member offset diff)
    ///
    /// Both operands must resolve to addresses within the same global symbol.
    /// The byte offsets are subtracted; for typed pointer subtraction the result
    /// is divided by the pointed-to element size.
    fn eval_const_ptr_diff(&self, lhs: &Expr, rhs: &Expr) -> Option<IrConst> {
        let lhs_addr = self.eval_global_addr_expr(lhs)?;
        let rhs_addr = self.eval_global_addr_expr(rhs)?;

        // Extract (symbol_name, byte_offset) from each side
        let (lhs_name, lhs_offset) = match &lhs_addr {
            GlobalInit::GlobalAddr(name) => (name.as_str(), 0i64),
            GlobalInit::GlobalAddrOffset(name, off) => (name.as_str(), *off),
            _ => return None,
        };
        let (rhs_name, rhs_offset) = match &rhs_addr {
            GlobalInit::GlobalAddr(name) => (name.as_str(), 0i64),
            GlobalInit::GlobalAddrOffset(name, off) => (name.as_str(), *off),
            _ => return None,
        };

        // Both must refer to the same global symbol
        if lhs_name != rhs_name {
            return None;
        }

        let byte_diff = lhs_offset - rhs_offset;

        // Determine if this is typed pointer subtraction (result in elements)
        // or byte/integer subtraction (result in bytes).
        // For typed pointer subtraction (ptr - ptr where both are non-void,
        // non-char pointers), divide by the element size.
        let result = if self.expr_is_pointer(lhs) && self.expr_is_pointer(rhs) {
            let elem_size = self.get_pointer_elem_size_from_expr(lhs) as i64;
            if elem_size > 1 {
                byte_diff / elem_size
            } else {
                byte_diff
            }
        } else {
            // Cast-to-integer subtraction: (long)&x - (long)&y, or
            // char*/void* subtraction: result is in bytes
            byte_diff
        };

        Some(IrConst::I64(result))
    }

    /// Convert an IrConst to i64. Delegates to IrConst::to_i64().
    pub(super) fn const_to_i64(&self, c: &IrConst) -> Option<i64> {
        c.to_i64()
    }

    /// The value of `val`, the folded initializer `e`, in a static object of
    /// IR type `dst_ir` (C type `dst_ct` when the caller knows it). Converts
    /// BY VALUE wherever the IR-type coercion would read carrier bits: a
    /// `_Bool`, decimal or `_Float128` destination, and a `_Float128`,
    /// `long double` or decimal source (`int i = 2.75Q` is 2, not the low
    /// bits of the binary128 pattern). Everything else keeps the IR coercion.
    pub(super) fn coerce_init_const(
        &self,
        val: IrConst,
        e: &Expr,
        dst_ir: IrType,
        dst_ct: Option<&CType>,
    ) -> IrConst {
        let src_ct = self.get_expr_ctype(e);
        let dst = dst_ct.cloned().or_else(|| scalar_ctype_of_ir(dst_ir));
        if let Some(c) = dst
            .as_ref()
            .and_then(|dst| Self::convert_const_by_value(&val, src_ct.as_ref(), dst, dst_ir))
        {
            return c;
        }
        val.coerce_to_with_src(dst_ir, Some(self.get_expr_type(e)))
    }

    /// The by-value half of [`Self::coerce_init_const`]; `None` when the IR
    /// coercion is already value-correct.
    fn convert_const_by_value(
        val: &IrConst,
        src_ct: Option<&CType>,
        dst: &CType,
        dst_ir: IrType,
    ) -> Option<IrConst> {
        if *dst == CType::Bool {
            return Some(IrConst::I8(i8::from(shared_const_eval::const_truth(
                val, src_ct,
            ))));
        }
        if dst.is_decimal() {
            let width = match dst {
                CType::Decimal32 => 32,
                CType::Decimal64 => 64,
                _ => 128,
            };
            return const_arith::const_to_bid(val, src_ct?, width);
        }
        if *dst == CType::Float128 {
            let bytes = shared_const_eval::f128_bytes_of(val, src_ct)?;
            return Some(IrConst::I128(u128::from_le_bytes(bytes) as i128));
        }
        let src_binary128 = match (src_ct, val) {
            (Some(CType::Float128), _) => shared_const_eval::f128_bytes_of(val, src_ct),
            (_, IrConst::LongDouble(_, bytes)) => {
                Some(crate::common::long_double::long_double_value_bytes(bytes))
            }
            _ => None,
        };
        if let Some(bytes) = src_binary128 {
            // A static initializer must be constant: out-of-range, NaN and
            // infinite values fold the way GCC folds them (saturation).
            return shared_const_eval::f128_to_ctype(&bytes, dst).map(|c| c.coerce_to(dst_ir));
        }
        if src_ct.is_some_and(CType::is_decimal) {
            return Self::cast_bid_to_ir_type(val, dst_ir, dst);
        }
        None
    }

    /// Coerce a constant to the target type, using the source expression's type for signedness.
    pub(super) fn coerce_const_to_type_with_src(
        &self,
        val: IrConst,
        target_ty: IrType,
        src_ty: IrType,
    ) -> IrConst {
        val.coerce_to_with_src(target_ty, Some(src_ty))
    }

    /// Collect array dimensions from nested Array type specifiers.
    /// Extract an integer value from any integer literal expression (Int, UInt, Long, ULong).
    /// Used for array sizes and other compile-time integer expressions.
    pub(super) fn expr_as_array_size(&self, expr: &Expr) -> Option<i64> {
        // Try simple literals first (fast path)
        match expr {
            Expr::IntLiteral(n, _) | Expr::LongLiteral(n, _) | Expr::LongLongLiteral(n, _) => {
                return Some(*n);
            }
            Expr::UIntLiteral(n, _) | Expr::ULongLiteral(n, _) | Expr::ULongLongLiteral(n, _) => {
                return Some(*n as i64);
            }
            _ => {}
        }
        // Fall back to full constant expression evaluation (handles sizeof, arithmetic, etc.)
        if let Some(val) = self.eval_const_expr(expr) {
            return self.const_to_i64(&val);
        }
        None
    }

    /// Array bound as a signed integer, validated *before* it can become a
    /// `usize`.
    ///
    /// These callers are the last place where the bound is still signed, so the
    /// two constraint violations that make the value unusable as a size have to
    /// be caught here:
    ///
    ///  * C11 6.7.6.2p5 requires a constant array bound to be greater than
    ///    zero. A negative value used to be `as usize`-wrapped into an
    ///    astronomically large element count, which turned a plain compile
    ///    error into a multi-gigabyte `.bss` reservation: with the bound
    ///    `sizeof(...) == 8 ? 1 : -1` the compiler walked into
    ///    "memory allocation of 17179869168 bytes failed" (SIGABRT) instead of
    ///    reporting the bad bound. Diagnose it.
    ///  * An array whose *byte* size exceeds `PTRDIFF_MAX` cannot exist,
    ///    because C11 6.5.6p9 requires pointer differences to be
    ///    representable; GCC rejects exactly that with "size of array is too
    ///    large" and so does this check. `elem_size` of 0 skips the byte test
    ///    for callers that only know the element count.
    ///
    /// Returns `None` for a rejected or non-constant bound, so callers keep
    /// their existing unsized-array behaviour; the diagnostic is what makes
    /// the compile fail with an explanation.
    pub(super) fn checked_array_bound(&self, expr: &Expr, elem_size: usize) -> Option<i64> {
        let n = self.expr_as_array_size(expr)?;
        if n < 0 {
            self.emit_error("size of array is negative", expr.span());
            return None;
        }
        if elem_size > 0 && (n as u128) * (elem_size as u128) > max_object_bytes() as u128 {
            self.emit_error("size of array is too large", expr.span());
            return None;
        }
        Some(n)
    }

    /// [`Self::checked_array_bound`] as an element count, for the callers that
    /// use the bound directly as a `usize`.
    pub(super) fn array_bound_elems(&self, expr: &Expr, elem_size: usize) -> Option<usize> {
        self.checked_array_bound(expr, elem_size)
            .map(|n| n as usize)
    }

    /// Check if an expression is always non-zero at compile time, even if we
    /// can't compute its exact numeric value.
    ///
    /// String literals, for example, are always non-null pointers, so they are
    /// always truthy in boolean context. This is needed for static initializers
    /// like `static const int NEED_OPTIONS = 0 || "Lcmwl" || 0;` (used by toybox).
    fn expr_is_always_nonzero(expr: &Expr) -> bool {
        match expr {
            Expr::StringLiteral(..)
            | Expr::WideStringLiteral(..)
            | Expr::Char16StringLiteral(..) => true,
            // A cast of a string literal is also always nonzero
            // (e.g., (int)"hello" in constant context)
            Expr::Cast(_, inner, _, _) => Self::expr_is_always_nonzero(inner),
            _ => false,
        }
    }

    /// True when `sizeof` of this argument is a runtime VLA size, not a
    /// compile-time constant. Used to keep `eval_const_expr` from folding
    /// `sizeof(vla_typedef) * N` into a pointer-sized constant.
    fn sizeof_arg_is_vla(&self, arg: &SizeofArg) -> bool {
        match arg {
            SizeofArg::Type(ts) => self.type_spec_is_vla(ts),
            SizeofArg::Expr(e) => self.expr_sizeof_is_vla(e),
        }
    }

    fn expr_sizeof_is_vla(&self, expr: &Expr) -> bool {
        if let Expr::Identifier(name, _) = expr {
            if let Some(fs) = self.func_state.as_ref() {
                if fs.vla_typedef_sizes.contains_key(name) {
                    return true;
                }
                if fs
                    .locals
                    .get(name)
                    .is_some_and(|info| info.vla_size.is_some())
                {
                    return true;
                }
            }
        }
        false
    }

    /// Recursively detect a variably-modified type (VLA array dimension, or
    /// a struct/union containing one, or a typedef of either).
    fn type_spec_is_vla(&self, ts: &TypeSpecifier) -> bool {
        if let Some(fs) = self.func_state.as_ref() {
            match ts {
                TypeSpecifier::TypedefName(name) => {
                    if fs.vla_typedef_sizes.contains_key(name) {
                        return true;
                    }
                }
                TypeSpecifier::Struct(Some(tag), ..) => {
                    if fs
                        .vla_typedef_sizes
                        .contains_key(&format!("struct.{}", tag))
                    {
                        return true;
                    }
                }
                TypeSpecifier::Union(Some(tag), ..) => {
                    if fs.vla_typedef_sizes.contains_key(&format!("union.{}", tag)) {
                        return true;
                    }
                }
                _ => {}
            }
        }
        let ts = self.resolve_type_spec(ts);
        match ts {
            TypeSpecifier::Array(elem, Some(size_expr)) => {
                self.expr_as_array_size(size_expr).is_none() || self.type_spec_is_vla(elem)
            }
            TypeSpecifier::Array(elem, None) => self.type_spec_is_vla(elem),
            TypeSpecifier::Struct(_, Some(fields), ..)
            | TypeSpecifier::Union(_, Some(fields), ..) => {
                fields.iter().any(|f| self.struct_field_is_vla(f))
            }
            TypeSpecifier::TypedefName(name) => {
                if let Some(ct) = self.types.typedefs.get(name) {
                    match ct {
                        crate::common::types::CType::Struct(key)
                        | crate::common::types::CType::Union(key) => self
                            .func_state
                            .as_ref()
                            .is_some_and(|fs| fs.vla_typedef_sizes.contains_key(key.as_ref())),
                        _ => false,
                    }
                } else {
                    false
                }
            }
            TypeSpecifier::Pointer(_, _) => false, // pointer-to-VLA is a pointer, sizeof is const
            TypeSpecifier::TypeofType(inner) => self.type_spec_is_vla(inner),
            _ => false,
        }
    }

    fn struct_field_is_vla(&self, field: &StructFieldDecl) -> bool {
        for d in &field.derived {
            if let DerivedDeclarator::Array(Some(expr)) = d {
                if self.expr_as_array_size(expr).is_none() {
                    return true;
                }
            }
        }
        self.type_spec_is_vla(&field.type_spec)
    }
}

/// The C scalar type an unambiguous IR storage type denotes, for by-value
/// constant conversion. 128-bit storage is ambiguous (`__int128`,
/// `_Float128`, `_Decimal128`) and so is a pointer; callers that may store
/// those pass the C type explicitly.
fn scalar_ctype_of_ir(ty: IrType) -> Option<CType> {
    Some(match ty {
        IrType::I8 => CType::Char,
        IrType::U8 => CType::UChar,
        IrType::I16 => CType::Short,
        IrType::U16 => CType::UShort,
        IrType::I32 => CType::Int,
        IrType::U32 => CType::UInt,
        IrType::I64 => CType::LongLong,
        IrType::U64 => CType::ULongLong,
        IrType::F32 => CType::Float,
        IrType::F64 => CType::Double,
        IrType::F128 => CType::LongDouble,
        _ => return None,
    })
}
