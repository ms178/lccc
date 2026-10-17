//! Shared constant expression evaluation functions.
//!
//! This module extracts the pure constant-evaluation logic that was duplicated
//! between `sema::const_eval` (SemaConstEval) and `ir::lowering::const_eval`
//! (Lowerer). Both evaluate compile-time constant expressions from the same AST,
//! but differ in how they resolve types:
//! - Sema uses CType (C-level types from the type checker)
//! - Lowering uses IrType (IR-level types from the lowerer)
//!
//! The functions here are parameterized by closures that abstract over these
//! differences, allowing both callers to share the same evaluation logic for:
//! - Literal evaluation
//! - Builtin function constant folding (__builtin_bswap, __builtin_clz, etc.)
//! - Sub-int promotion for unary operations
//! - Bit-width evaluation through cast chains
//!
//! Functions that require caller-specific state (global address resolution,
//! sizeof/alignof, binary operations with type inference) remain in the
//! respective callers.

use super::const_arith;
use super::types::{CType, target_is_32bit, target_ptr_size};
use crate::frontend::parser::ast::{BinOp, Expr, TypeSpecifier};
use crate::ir::reexports::IrConst;

/// Evaluate a literal expression to an IrConst.
/// Returns None for non-literal expressions.
#[inline]
pub fn eval_literal(expr: &Expr) -> Option<IrConst> {
    match expr {
        // IntLiteral is `int` (32-bit) when the value fits, otherwise `long`.
        Expr::IntLiteral(val, _) => {
            if *val >= i32::MIN as i64 && *val <= i32::MAX as i64 {
                Some(IrConst::I32(*val as i32))
            } else {
                Some(IrConst::I64(*val))
            }
        }
        Expr::LongLiteral(val, _) => Some(IrConst::I64(*val)),
        Expr::LongLongLiteral(val, _) => Some(IrConst::I64(*val)),
        // UIntLiteral stays as I64 to preserve the unsigned value.
        Expr::UIntLiteral(val, _) => Some(IrConst::I64(*val as i64)),
        Expr::ULongLiteral(val, _) => Some(IrConst::I64(*val as i64)),
        Expr::ULongLongLiteral(val, _) => Some(IrConst::I64(*val as i64)),
        Expr::CharLiteral(ch, _) => {
            // Sign-extend from signed char to int, matching GCC behavior.
            // '\xEF' should be -17, not 239, when char is signed.
            Some(IrConst::I32(*ch as u8 as i8 as i32))
        }
        Expr::FloatLiteral(val, _) => Some(IrConst::F64(*val)),
        Expr::FloatLiteralF32(val, _) => Some(IrConst::F32(*val as f32)),
        Expr::FloatLiteralLongDouble(val, bytes, _) => {
            Some(IrConst::long_double_with_bytes(*val, *bytes))
        }
        Expr::FloatLiteralF128(_, bytes, _) => {
            // _Float128 literal: the 16 bytes ARE the IEEE binary128 value,
            // carried bit-exact in an I128 constant.
            Some(IrConst::I128(u128::from_le_bytes(*bytes) as i128))
        }
        _ => None,
    }
}

/// Return the logical C length of an LCCC-decoded narrow string literal.
///
/// Narrow literals are represented as Rust chars rather than UTF-8 bytes, so
/// `str::len()` is not the compiler's existing execution-character length
/// model. The C string length ends at the first embedded NUL, even though the
/// literal's array object can contain later characters.
pub(crate) fn narrow_string_strlen(text: &str) -> usize {
    text.chars().take_while(|&c| c != '\0').count()
}

/// Whether `__builtin_constant_p(expr)` is 1 because `expr` is the address of
/// a string literal, by GCC 16.2's rule (`fold_builtin_constant_p`, measured
/// in static initializers): after stripping no-op conversions, the argument
/// is a string literal, `&"s"`, `&"s"[0]`, or a conditional whose constant
/// condition selects one. Pointer arithmetic (`"s" + 1`), dereferences
/// (`"s"[0]`), the comma operator and `__func__` are not (GCC folds them to 0).
///
/// `cast_is_nop` decides whether a cast is a no-op conversion of a pointer:
/// to a pointer type, or to an integer type as wide as a pointer
/// (`(long)"s"` is 1, `(char)"s"` and, on i686, `(long long)"s"` are 0).
pub fn is_string_literal_address(
    expr: &Expr,
    cast_is_nop: &dyn Fn(&TypeSpecifier) -> bool,
    eval_fn: &dyn Fn(&Expr) -> Option<IrConst>,
) -> bool {
    let is_literal = |e: &Expr| {
        matches!(
            e,
            Expr::StringLiteral(..) | Expr::WideStringLiteral(..) | Expr::Char16StringLiteral(..)
        )
    };
    match expr {
        e if is_literal(e) => true,
        Expr::Cast(target, inner, ..) => {
            cast_is_nop(target) && is_string_literal_address(inner, cast_is_nop, eval_fn)
        }
        Expr::AddressOf(inner, _) => match inner.as_ref() {
            e if is_literal(e) => true,
            Expr::ArraySubscript(base, index, _) => {
                is_literal(base) && eval_fn(index).is_some_and(|i| i.is_zero())
            }
            _ => false,
        },
        Expr::Conditional(cond, then, els, _) => eval_fn(cond).is_some_and(|c| {
            let taken = if c.is_nonzero() { then } else { els };
            is_string_literal_address(taken, cast_is_nop, eval_fn)
        }),
        _ => false,
    }
}

/// [`is_string_literal_address`]'s no-op test for casts spelled with builtin
/// type names (typedefs need type resolution, which the sema evaluator has).
fn builtin_cast_is_nop(target: &TypeSpecifier) -> bool {
    let int_size = match target {
        TypeSpecifier::Pointer(..) => return true,
        TypeSpecifier::Long | TypeSpecifier::UnsignedLong => target_ptr_size(),
        TypeSpecifier::LongLong | TypeSpecifier::UnsignedLongLong => 8,
        _ => return false,
    };
    int_size == target_ptr_size()
}

/// Evaluate a compile-time builtin function call in a constant expression.
///
/// Handles all pure builtins that can be folded at compile time:
/// - __builtin_choose_expr, __builtin_constant_p, __builtin_expect
/// - __builtin_bswap{16,32,64}
/// - __builtin_clz{,l,ll}, __builtin_ctz{,l,ll}
/// - __builtin_popcount{,l,ll}, __builtin_ffs{,l,ll}
/// - __builtin_parity{,l,ll}, __builtin_clrsb{,l,ll}
///
/// `eval_fn` is the caller's recursive eval_const_expr function.
/// Evaluate a compile-time builtin call in hosted mode (non-prefixed
/// hosted-library names fold).  Thin wrapper over
/// [`eval_builtin_call_with_mode`] with the default policy; kept as the
/// natural entry point for contexts without a builtin-policy thread.
pub fn eval_builtin_call(
    name: &str,
    args: &[Expr],
    eval_fn: &dyn Fn(&Expr) -> Option<IrConst>,
) -> Option<IrConst> {
    eval_builtin_call_with_mode(name, args, eval_fn, true)
}

/// Evaluate a compile-time builtin call under an explicit builtin policy.
/// `allow_nonprefixed_builtins=false` (`-fno-builtin` / `-ffreestanding`)
/// withdraws EVERY non-`__`-prefixed spelling — the whole bare-name fold
/// table (strlen, fabs, floor, ceil, fma, fmax, fmin, copysign, ...) consists
/// of hosted-library aliases; the reserved `__builtin_*`/`__lccc_*` spellings
/// stay available in both hosted and freestanding modes (C11 7.1.3 reserved
/// identifiers). Pinning: `check_fno_builtin_folds.sh`.
pub fn eval_builtin_call_with_mode(
    name: &str,
    args: &[Expr],
    eval_fn: &dyn Fn(&Expr) -> Option<IrConst>,
    allow_nonprefixed_builtins: bool,
) -> Option<IrConst> {
    if !allow_nonprefixed_builtins && !name.starts_with("__") {
        return None;
    }
    match name {
        "__builtin_strlen" | "strlen" => {
            // String literal -> compile-time length (without the NUL).
            // glibc startup.h: __builtin_constant_p(__builtin_strlen(msg)).
            // Narrow literals are stored one C byte per Rust char (see
            // lex_string), so the byte length is chars().count() — String::len()
            // returns the UTF-8 encoded size and over-counts bytes >= 0x80
            // (expat symptom: strlen("\xEF\xBB\xBF...") folded 3 too long).
            if let Some(crate::frontend::parser::ast::Expr::StringLiteral(text, _)) = args.first() {
                Some(IrConst::I64(narrow_string_strlen(text) as i64))
            } else {
                None
            }
        }
        "__builtin_choose_expr" if args.len() >= 3 => {
            let cond = eval_fn(&args[0])?;
            if cond.is_nonzero() {
                eval_fn(&args[1])
            } else {
                eval_fn(&args[2])
            }
        }
        "__builtin_constant_p" => {
            // Prove positive answers early, but do not cache an early zero.
            // A local/parameter may become constant after mem2reg, inlining,
            // and propagation; the IR IsConstant node must survive until then.
            if args.first().is_some_and(|arg| {
                eval_fn(arg).is_some()
                    || is_string_literal_address(arg, &builtin_cast_is_nop, eval_fn)
            }) {
                Some(IrConst::I32(1))
            } else {
                None
            }
        }
        "__builtin_expect" | "__builtin_expect_with_probability" => args.first().and_then(eval_fn),
        "__builtin_bswap16" => {
            let val = eval_fn(args.first()?)?;
            let v = val.to_i64()? as u16;
            Some(IrConst::I32(v.swap_bytes() as i32))
        }
        "__builtin_bswap32" => {
            let val = eval_fn(args.first()?)?;
            let v = val.to_i64()? as u32;
            Some(IrConst::I32(v.swap_bytes() as i32))
        }
        "__builtin_bswap64" => {
            let val = eval_fn(args.first()?)?;
            let v = val.to_i64()? as u64;
            Some(IrConst::I64(v.swap_bytes() as i64))
        }
        // For 'l' suffix builtins, the operand width depends on the target:
        // LP64 (x86-64, ARM64, RISC-V 64): long is 64-bit
        // ILP32 (i686): long is 32-bit
        "__builtin_clz" => {
            let val = eval_fn(args.first()?)?;
            let v = val.to_i64()? as u32;
            Some(IrConst::I32(v.leading_zeros() as i32))
        }
        "__builtin_clzl" => {
            let val = eval_fn(args.first()?)?;
            if target_is_32bit() {
                let v = val.to_i64()? as u32;
                Some(IrConst::I32(v.leading_zeros() as i32))
            } else {
                let v = val.to_i64()? as u64;
                Some(IrConst::I32(v.leading_zeros() as i32))
            }
        }
        "__builtin_clzll" => {
            let val = eval_fn(args.first()?)?;
            let v = val.to_i64()? as u64;
            Some(IrConst::I32(v.leading_zeros() as i32))
        }
        "__builtin_ctz" => {
            let val = eval_fn(args.first()?)?;
            let v = val.to_i64()? as u32;
            if v == 0 {
                Some(IrConst::I32(32))
            } else {
                Some(IrConst::I32(v.trailing_zeros() as i32))
            }
        }
        "__builtin_ctzl" => {
            let val = eval_fn(args.first()?)?;
            if target_is_32bit() {
                let v = val.to_i64()? as u32;
                if v == 0 {
                    Some(IrConst::I32(32))
                } else {
                    Some(IrConst::I32(v.trailing_zeros() as i32))
                }
            } else {
                let v = val.to_i64()? as u64;
                if v == 0 {
                    Some(IrConst::I32(64))
                } else {
                    Some(IrConst::I32(v.trailing_zeros() as i32))
                }
            }
        }
        "__builtin_ctzll" => {
            let val = eval_fn(args.first()?)?;
            let v = val.to_i64()? as u64;
            if v == 0 {
                Some(IrConst::I32(64))
            } else {
                Some(IrConst::I32(v.trailing_zeros() as i32))
            }
        }
        "__builtin_popcount" => {
            let val = eval_fn(args.first()?)?;
            let v = val.to_i64()? as u32;
            Some(IrConst::I32(v.count_ones() as i32))
        }
        "__builtin_popcountl" => {
            let val = eval_fn(args.first()?)?;
            if target_is_32bit() {
                let v = val.to_i64()? as u32;
                Some(IrConst::I32(v.count_ones() as i32))
            } else {
                let v = val.to_i64()? as u64;
                Some(IrConst::I32(v.count_ones() as i32))
            }
        }
        "__builtin_popcountll" => {
            let val = eval_fn(args.first()?)?;
            let v = val.to_i64()? as u64;
            Some(IrConst::I32(v.count_ones() as i32))
        }
        "__builtin_ffs" => {
            let val = eval_fn(args.first()?)?;
            let v = val.to_i64()? as u32;
            if v == 0 {
                Some(IrConst::I32(0))
            } else {
                Some(IrConst::I32(v.trailing_zeros() as i32 + 1))
            }
        }
        "__builtin_ffsl" => {
            let val = eval_fn(args.first()?)?;
            if target_is_32bit() {
                let v = val.to_i64()? as u32;
                if v == 0 {
                    Some(IrConst::I32(0))
                } else {
                    Some(IrConst::I32(v.trailing_zeros() as i32 + 1))
                }
            } else {
                let v = val.to_i64()? as u64;
                if v == 0 {
                    Some(IrConst::I32(0))
                } else {
                    Some(IrConst::I32(v.trailing_zeros() as i32 + 1))
                }
            }
        }
        "__builtin_ffsll" => {
            let val = eval_fn(args.first()?)?;
            let v = val.to_i64()? as u64;
            if v == 0 {
                Some(IrConst::I32(0))
            } else {
                Some(IrConst::I32(v.trailing_zeros() as i32 + 1))
            }
        }
        "__builtin_parity" => {
            let val = eval_fn(args.first()?)?;
            let v = val.to_i64()? as u32;
            Some(IrConst::I32((v.count_ones() % 2) as i32))
        }
        "__builtin_parityl" => {
            let val = eval_fn(args.first()?)?;
            if target_is_32bit() {
                let v = val.to_i64()? as u32;
                Some(IrConst::I32((v.count_ones() % 2) as i32))
            } else {
                let v = val.to_i64()? as u64;
                Some(IrConst::I32((v.count_ones() % 2) as i32))
            }
        }
        "__builtin_parityll" => {
            let val = eval_fn(args.first()?)?;
            let v = val.to_i64()? as u64;
            Some(IrConst::I32((v.count_ones() % 2) as i32))
        }
        "__builtin_clrsb" => {
            let val = eval_fn(args.first()?)?;
            let v = val.to_i64()? as i32;
            let result = if v < 0 {
                (!v as u32).leading_zeros() as i32 - 1
            } else {
                (v as u32).leading_zeros() as i32 - 1
            };
            Some(IrConst::I32(result))
        }
        "__builtin_clrsbl" => {
            let val = eval_fn(args.first()?)?;
            if target_is_32bit() {
                let v = val.to_i64()? as i32;
                let result = if v < 0 {
                    (!v as u32).leading_zeros() as i32 - 1
                } else {
                    (v as u32).leading_zeros() as i32 - 1
                };
                Some(IrConst::I32(result))
            } else {
                let v = val.to_i64()?;
                let result = if v < 0 {
                    (!v as u64).leading_zeros() as i32 - 1
                } else {
                    (v as u64).leading_zeros() as i32 - 1
                };
                Some(IrConst::I32(result))
            }
        }
        "__builtin_clrsbll" => {
            let val = eval_fn(args.first()?)?;
            let v = val.to_i64()?;
            let result = if v < 0 {
                (!v as u64).leading_zeros() as i32 - 1
            } else {
                (v as u64).leading_zeros() as i32 - 1
            };
            Some(IrConst::I32(result))
        }
        // ------------------------------------------------------------------
        // The math library, folded exactly where GCC folds it
        // ------------------------------------------------------------------
        //
        // GCC answers `sqrt(4.0)`, `__builtin_isnan(1.0)`, `__builtin_fma(…)`
        // and friends from its own MPFR evaluator whenever every argument is a
        // constant, in `static double y = __builtin_sqrt(4.0);` as much as in
        // an `enum` or `case` context.  Every arm below therefore computes in
        // the precision the emitted call would use — rustc's `f32`/`f64`
        // elementary operations are the IEEE operations (correctly rounded,
        // `f64::mul_add` the fused one) — so a folded constant and the runtime
        // answer cannot disagree.
        //
        // Two rules, both measured against GCC 16.2 and GCC 12.2 (identical
        // verdicts) and pinned by tests/regression/file_scope_init/values.c:
        //
        //   * GCC folds the bit-level and classification builtins (`fabs`,
        //     `copysign`, `isnan`, `isinf`, `isfinite`, `isnormal`,
        //     `fpclassify`) for *any* constant argument, infinities and NaNs
        //     included: `__builtin_fabs(1.0/0.0)` is a constant.
        //     `__builtin_signbit` is the exception -- it is finite-only.
        //   * The operations GCC computes with MPFR (`sqrt`, `round`, `fmin`,
        //     `fmax`, `fma`, `lround`, `llround`, `lrint`, `llrint`) fold only
        //     for finite arguments and a finite result: GCC rejects
        //     `__builtin_sqrt(-1.0)` (NaN result), `__builtin_sqrt(1.0/0.0)`,
        //     `__builtin_fmin(1.0, 1.0/0.0)` and
        //     `__builtin_round(1e308*10.0)` as "initializer element is not
        //     constant", and so does this.
        //   * The integral operations GCC computes in `real.c` (`floor`,
        //     `ceil`, `trunc`) and the bit-level `copysign`/`signbit` fold for
        //     *any* value a literal or a builtin wrote down, non-finite ones
        //     included (`__builtin_floor(__builtin_inf())` and
        //     `__builtin_signbit(-__builtin_inf())` are constants), but not for
        //     a non-finite value that arithmetic produced (`1.0/0.0`,
        //     `1e308*10.0`): see `nonfinite_from_arithmetic`.
        //
        // The `l` forms fold only when the exact answer is an `f64`
        // (`fabsl(2.5L)`), because lccc carries a long double constant as
        // `(f64 approximation, exact binary128 bytes)`: an inexact result
        // computed at f64 precision (`sqrtl(2.0L)`) is a *different number*
        // than the target's correctly-rounded long double arithmetic.
        // `first_f64` is what enforces this: a long double argument folds only
        // when its own bits are the exact expansion of its `f64` field.
        "__builtin_sqrt" | "sqrt" => rounded_f64(args, eval_fn, f64::sqrt),
        "__builtin_sqrtf" | "sqrtf" => rounded_f32(args, eval_fn, f32::sqrt),
        "__builtin_sqrtl" | "sqrtl" => {
            let x = first_f64(args, eval_fn)?;
            let r = x.sqrt();
            (r.is_finite() && r * r == x).then(|| IrConst::long_double(r))
        }
        "__builtin_fabs" | "fabs" => bit_f64(args, eval_fn, f64::abs),
        "__builtin_fabsf" | "fabsf" => bit_f32(args, eval_fn, f32::abs),
        "__builtin_fabsl" | "fabsl" => long_double_exact(args, eval_fn, f64::abs),
        "__builtin_copysign" | "copysign" => copysign_f64(args, eval_fn),
        "__builtin_copysignf" | "copysignf" => copysign_f32(args, eval_fn),
        "__builtin_copysignl" | "copysignl" => copysign_long_double(args, eval_fn),
        "__builtin_floor" | "floor" => integral_f64(args, eval_fn, f64::floor),
        "__builtin_floorf" | "floorf" => integral_f32(args, eval_fn, f32::floor),
        "__builtin_floorl" | "floorl" => integral_long_double(args, eval_fn, f64::floor),
        "__builtin_ceil" | "ceil" => integral_f64(args, eval_fn, f64::ceil),
        "__builtin_ceilf" | "ceilf" => integral_f32(args, eval_fn, f32::ceil),
        "__builtin_ceill" | "ceill" => integral_long_double(args, eval_fn, f64::ceil),
        "__builtin_trunc" | "trunc" => integral_f64(args, eval_fn, f64::trunc),
        "__builtin_truncf" | "truncf" => integral_f32(args, eval_fn, f32::trunc),
        "__builtin_truncl" | "truncl" => integral_long_double(args, eval_fn, f64::trunc),
        "__builtin_round" | "round" => rounded_f64(args, eval_fn, f64::round),
        "__builtin_roundf" | "roundf" => rounded_f32(args, eval_fn, f32::round),
        "__builtin_roundl" | "roundl" => long_double_rounded(args, eval_fn, f64::round),
        "__builtin_fmax" | "fmax" => rounded_f64_2(args, eval_fn, f64::max),
        "__builtin_fmaxf" | "fmaxf" => rounded_f32_2(args, eval_fn, f32::max),
        "__builtin_fmaxl" | "fmaxl" => long_double_rounded_2(args, eval_fn, f64::max),
        "__builtin_fmin" | "fmin" => rounded_f64_2(args, eval_fn, f64::min),
        "__builtin_fminf" | "fminf" => rounded_f32_2(args, eval_fn, f32::min),
        "__builtin_fminl" | "fminl" => long_double_rounded_2(args, eval_fn, f64::min),
        "__builtin_fma" | "fma" => {
            let (a, b, c) = finite_f64_triple(args, eval_fn)?;
            let r = a.mul_add(b, c);
            r.is_finite().then(|| IrConst::F64(r))
        }
        "__builtin_fmaf" | "fmaf" => {
            let args3 = args.get(..3)?;
            let a = first_f32(&args3[0..1], eval_fn)?;
            let b = first_f32(&args3[1..2], eval_fn)?;
            let c = first_f32(&args3[2..3], eval_fn)?;
            let r = a.mul_add(b, c);
            (a.is_finite() && b.is_finite() && c.is_finite() && r.is_finite())
                .then(|| IrConst::F32(r))
        }
        // Integer-valued conversions: the value is exact, the target type is
        // the declared one, so no rounding rule is involved beyond the mode.
        // `lround` rounds half away from zero.  `lrint`/`llrint` are *not*
        // folded at all: they follow the current rounding mode, and GCC emits
        // a call to them even inside a function body (measured), so a constant
        // here would accept programs GCC rejects.
        "__builtin_lround" | "lround" => rounded_i64(args, eval_fn, f64::round),
        "__builtin_lroundf" | "lroundf" => rounded_i64_f32(args, eval_fn, f32::round),
        "__builtin_llround" | "llround" => rounded_i64(args, eval_fn, f64::round),
        "__builtin_llroundf" | "llroundf" => rounded_i64_f32(args, eval_fn, f32::round),
        // Classification: the answer is an `int` and exists for every
        // argument, including the non-finite ones.
        "__builtin_isnan" | "isnan" => classified_f64(args, eval_fn, f64::is_nan),
        "__builtin_isinf" | "isinf" => classified_f64(args, eval_fn, f64::is_infinite),
        "__builtin_isfinite" | "isfinite" => classified_f64(args, eval_fn, f64::is_finite),
        "__builtin_isnormal" | "isnormal" => classified_f64(args, eval_fn, f64::is_normal),
        "__builtin_signbit" | "signbit" => {
            let arg = args.first()?;
            let x = first_f64(args, eval_fn)?;
            if !x.is_finite() && nonfinite_from_arithmetic(arg, eval_fn) {
                return None;
            }
            Some(IrConst::I32(x.is_sign_negative() as i32))
        }
        // `__builtin_fpclassify(nan, inf, normal, subnormal, zero, x)`: the
        // argument order is the one glibc's `fpclassify` uses, and it is
        // measured, not assumed — `__builtin_fpclassify(10,11,12,13,14,0.0)`
        // is 14 and the same macro with `1.0/0.0` is 11 on GCC 16.2.
        "__builtin_fpclassify" => {
            let class = args.get(0..6)?;
            let x = first_f64(&class[5..6], eval_fn)?;
            let index = if x.is_nan() {
                0
            } else if x.is_infinite() {
                1
            } else if x.is_normal() {
                2
            } else if x.is_subnormal() {
                3
            } else {
                4 // zero and negative zero
            };
            eval_fn(&class[index])
        }

        // Float constant builtins: NaN, Infinity, huge_val

        // These are critical for static/global initializers like:
        //   volatile double d = NAN;  // expands to __builtin_nan("")
        //   double inf = INFINITY;    // expands to __builtin_inff()
        // Without these, eval_const_expr returns None and global_init.rs
        // falls through to GlobalInit::Zero, silently zero-initializing
        // the variable instead of storing NaN/Infinity.
        "__builtin_nan" | "nan" => Some(nan_f64(nan_payload(args)?, false)),
        "__builtin_nanf" | "nanf" => Some(nan_f32(nan_payload(args)?, false)),
        "__builtin_inf" => Some(IrConst::F64(f64::INFINITY)),
        "__builtin_inff" => Some(IrConst::F32(f32::INFINITY)),
        "__builtin_infl" => Some(IrConst::long_double(f64::INFINITY)),
        "__builtin_huge_val" => Some(IrConst::F64(f64::INFINITY)),
        "__builtin_huge_valf" => Some(IrConst::F32(f32::INFINITY)),
        "__builtin_huge_vall" => Some(IrConst::long_double(f64::INFINITY)),
        // `__builtin_nans`: the signaling NaN.  Its payload bits are the
        // argument string's value (measured: `nans("7")` is
        // 0x7ff0000000000007 -- no quiet bit), and the canonical signaling NaN
        // for an empty payload is 0x7ff4000000000000 / 0x7fa00000.
        "__builtin_nans" => Some(nan_f64(nan_payload(args)?, true)),
        "__builtin_nansf" => Some(nan_f32(nan_payload(args)?, true)),
        // A long double NaN keeps its canonical bit pattern: the payload of
        // `__builtin_nanl("7")` lives in the x87 significand
        // (0xc000000000000007) while lccc carries a long double as
        // `(f64 approximation, binary128 bytes)`, so the *value* is exact (a
        // NaN) but its payload is not preserved.  The alternative -- declining
        // -- would reject a program GCC accepts, which is worse.
        "__builtin_nanl" | "nanl" => {
            let _ = nan_payload(args)?;
            Some(IrConst::long_double(f64::NAN))
        }
        "__builtin_nansl" | "nansl" => {
            let _ = nan_payload(args)?;
            Some(IrConst::long_double(f64::from_bits(0x7ff4_0000_0000_0000)))
        }
        _ => None,
    }
}

/// The payload of a `__builtin_nan*`/`__builtin_nans*` argument.
///
/// The string is parsed the way GCC parses it -- every rule below is measured
/// against GCC 16.2: C integer bases (`0x` hex, leading `0` octal, otherwise
/// decimal), a sign gives the magnitude only (`nan("-1")` has payload 1),
/// leading whitespace is skipped (`nan("\t7")` has payload 7), trailing
/// characters are rejected (`nan("12abc")` and `nan("7 ")` are not constants,
/// GCC reports "initializer element is not constant"), and a value wider than
/// the mantissa is truncated (`nan("0x10000000000000")` is the canonical quiet
/// NaN).  `nan("")`, `nan(" ")` and `nan("0x")` are payload 0.
///
/// `None` means the call is not a constant.
fn nan_payload(args: &[Expr]) -> Option<u64> {
    let text = match args.first() {
        None => return Some(0),
        Some(Expr::StringLiteral(text, _)) => text.as_str(),
        Some(_) => return None,
    };
    let rest = text.trim_start();
    if rest.is_empty() {
        return Some(0);
    }
    let rest = rest
        .strip_prefix('+')
        .or_else(|| rest.strip_prefix('-'))
        .unwrap_or(rest);
    if let Some(hex) = rest.strip_prefix("0x").or_else(|| rest.strip_prefix("0X")) {
        // `nan("0x")`: a radix with no digits is payload 0 (measured).
        return if hex.is_empty() {
            Some(0)
        } else {
            nan_payload_digits(hex, 16)
        };
    }
    if let Some(octal) = rest.strip_prefix('0') {
        // The leading `0` is the base marker; it contributes no value.
        return if octal.is_empty() {
            Some(0)
        } else {
            nan_payload_digits(octal, 8)
        };
    }
    nan_payload_digits(rest, 10)
}

/// Accumulate `digits` in `radix`, discarding anything above 64 bits the way
/// GCC truncates a too-wide payload, or `None` when a character is not a digit
/// of that radix.
fn nan_payload_digits(digits: &str, radix: u32) -> Option<u64> {
    let mut value: u64 = 0;
    for ch in digits.chars() {
        let digit = ch.to_digit(radix)?;
        value = value
            .wrapping_mul(u64::from(radix))
            .wrapping_add(u64::from(digit));
    }
    Some(value)
}

/// A `double` NaN carrying `payload`: quiet for `__builtin_nan`, signaling for
/// `__builtin_nans`, with GCC's canonical bit patterns for an empty payload.
fn nan_f64(payload: u64, signaling: bool) -> IrConst {
    const MANTISSA: u64 = 0x000f_ffff_ffff_ffff;
    const QUIET: u64 = 0x0008_0000_0000_0000;
    const CANONICAL_SIGNALING: u64 = 0x0004_0000_0000_0000;
    let bits = if signaling {
        match payload {
            0 => 0x7ff0_0000_0000_0000 | CANONICAL_SIGNALING,
            p => 0x7ff0_0000_0000_0000 | (p & MANTISSA),
        }
    } else {
        0x7ff0_0000_0000_0000 | QUIET | (payload & MANTISSA)
    };
    IrConst::F64(f64::from_bits(bits))
}

/// [`nan_f64`] in `float` precision.
fn nan_f32(payload: u64, signaling: bool) -> IrConst {
    const MANTISSA: u64 = 0x007f_ffff;
    const QUIET: u64 = 0x0040_0000;
    const CANONICAL_SIGNALING: u64 = 0x0020_0000;
    let bits = if signaling {
        match payload {
            0 => 0x7f80_0000 | CANONICAL_SIGNALING,
            p => 0x7f80_0000 | (p & MANTISSA),
        }
    } else {
        0x7f80_0000 | QUIET | (payload & MANTISSA)
    };
    IrConst::F32(f32::from_bits(bits as u32))
}

/// The `f64` value of `val`, when `val` *is* that value exactly: an integer
/// that fits in `f64`, an `f32` (every `f32` is an `f64`), an `f64`, or a long
/// double whose stored bytes are the exact expansion of its `f64` field.  A
/// long double that carries precision beyond `f64` (`1.0L/3.0L`) answers
/// `None`: any value computed from it at `f64` precision would differ from the
/// target's own long double arithmetic.
fn exactly_f64(val: &IrConst) -> Option<f64> {
    match *val {
        IrConst::Zero => Some(0.0),
        IrConst::F32(v) => Some(f64::from(v)),
        IrConst::F64(v) => Some(v),
        IrConst::I8(v) => Some(f64::from(v)),
        IrConst::I16(v) => Some(f64::from(v)),
        IrConst::I32(v) => Some(f64::from(v)),
        IrConst::I64(v) if v as f64 as i64 == v => Some(v as f64),
        // An integer no `f64` can hold exactly, a 128-bit integer, or a
        // decimal float: not an `f64` value, so not foldable as one.
        IrConst::I64(_) | IrConst::I128(_) | IrConst::D32(_) | IrConst::D64(_) => None,
        IrConst::LongDouble(v, bytes) => {
            (crate::common::long_double::f64_to_f128_bytes_lossless(v) == bytes).then_some(v)
        }
    }
}

/// The first argument as an exact `f64` (see [`exactly_f64`]).
fn first_f64(args: &[Expr], eval_fn: &dyn Fn(&Expr) -> Option<IrConst>) -> Option<f64> {
    exactly_f64(&eval_fn(args.first()?)?)
}

/// The first argument as an `f32`.  A `float` argument rounds once (the C
/// conversion), and the operation itself then runs in `f32` so the folded
/// constant is the correctly-rounded `float` result rather than an `f64`
/// result that was rounded twice.
fn first_f32(args: &[Expr], eval_fn: &dyn Fn(&Expr) -> Option<IrConst>) -> Option<f32> {
    Some(first_f64(args, eval_fn)? as f32)
}

/// A correctly-rounded `f64` operation: GCC folds it for finite arguments and
/// a finite result only (see the block comment in [`eval_builtin_call`]).
fn rounded_f64(
    args: &[Expr],
    eval_fn: &dyn Fn(&Expr) -> Option<IrConst>,
    f: fn(f64) -> f64,
) -> Option<IrConst> {
    let x = first_f64(args, eval_fn)?;
    let r = f(x);
    (x.is_finite() && r.is_finite()).then(|| IrConst::F64(r))
}

/// [`rounded_f64`] in `float` precision.
fn rounded_f32(
    args: &[Expr],
    eval_fn: &dyn Fn(&Expr) -> Option<IrConst>,
    f: fn(f32) -> f32,
) -> Option<IrConst> {
    let x = first_f32(args, eval_fn)?;
    let r = f(x);
    (x.is_finite() && r.is_finite()).then(|| IrConst::F32(r))
}

/// A two-argument correctly-rounded `f64` operation (`fmin`, `fmax`).
fn rounded_f64_2(
    args: &[Expr],
    eval_fn: &dyn Fn(&Expr) -> Option<IrConst>,
    f: fn(f64, f64) -> f64,
) -> Option<IrConst> {
    let (a, b) = (first_f64(args, eval_fn)?, first_f64(&args[1..], eval_fn)?);
    let r = f(a, b);
    (a.is_finite() && b.is_finite() && r.is_finite()).then(|| IrConst::F64(r))
}

/// [`rounded_f64_2`] in `float` precision.
fn rounded_f32_2(
    args: &[Expr],
    eval_fn: &dyn Fn(&Expr) -> Option<IrConst>,
    f: fn(f32, f32) -> f32,
) -> Option<IrConst> {
    let (a, b) = (first_f32(args, eval_fn)?, first_f32(&args[1..], eval_fn)?);
    let r = f(a, b);
    (a.is_finite() && b.is_finite() && r.is_finite()).then(|| IrConst::F32(r))
}

/// The three `f64` operands of `__builtin_fma`, all finite.
fn finite_f64_triple(
    args: &[Expr],
    eval_fn: &dyn Fn(&Expr) -> Option<IrConst>,
) -> Option<(f64, f64, f64)> {
    let (a, b, c) = (
        first_f64(args, eval_fn)?,
        first_f64(&args[1..], eval_fn)?,
        first_f64(&args[2..], eval_fn)?,
    );
    (a.is_finite() && b.is_finite() && c.is_finite()).then_some((a, b, c))
}

/// An exact `f64` operation whose result is `float`: `fabs`, `copysign`.  The
/// bit-level operations fold for every value, infinities and NaNs included.
fn bit_f32(
    args: &[Expr],
    eval_fn: &dyn Fn(&Expr) -> Option<IrConst>,
    f: fn(f32) -> f32,
) -> Option<IrConst> {
    Some(IrConst::F32(f(first_f32(args, eval_fn)?)))
}

/// [`bit_f32`] in `double` precision.
fn bit_f64(
    args: &[Expr],
    eval_fn: &dyn Fn(&Expr) -> Option<IrConst>,
    f: fn(f64) -> f64,
) -> Option<IrConst> {
    Some(IrConst::F64(f(first_f64(args, eval_fn)?)))
}

/// Two-argument bit-level operation (`copysign`).
fn bit_f64_2(
    args: &[Expr],
    eval_fn: &dyn Fn(&Expr) -> Option<IrConst>,
    f: fn(f64, f64) -> f64,
) -> Option<IrConst> {
    Some(IrConst::F64(f(
        first_f64(args, eval_fn)?,
        first_f64(&args[1..], eval_fn)?,
    )))
}

/// [`bit_f64_2`] in `float` precision.
fn bit_f32_2(
    args: &[Expr],
    eval_fn: &dyn Fn(&Expr) -> Option<IrConst>,
    f: fn(f32, f32) -> f32,
) -> Option<IrConst> {
    Some(IrConst::F32(f(
        first_f32(args, eval_fn)?,
        first_f32(&args[1..], eval_fn)?,
    )))
}

/// A `long double` result that is *exact*: the argument is already an exact
/// `f64` and the operation maps `f64` values to `f64` values exactly
/// (`fabs`, `floor`, `ceil`, `trunc`, `round`, `copysign`).  The value is then
/// the same number the target's own long double arithmetic produces.
fn long_double_exact(
    args: &[Expr],
    eval_fn: &dyn Fn(&Expr) -> Option<IrConst>,
    f: fn(f64) -> f64,
) -> Option<IrConst> {
    Some(IrConst::long_double(f(first_f64(args, eval_fn)?)))
}

/// [`long_double_exact`] for two arguments.
fn long_double_exact_2(
    args: &[Expr],
    eval_fn: &dyn Fn(&Expr) -> Option<IrConst>,
    f: fn(f64, f64) -> f64,
) -> Option<IrConst> {
    Some(IrConst::long_double(f(
        first_f64(args, eval_fn)?,
        first_f64(&args[1..], eval_fn)?,
    )))
}

/// [`long_double_exact`] for the operations GCC folds for finite operands
/// only: the exactness of the *operation* is not enough, the folded value must
/// also be finite (`floorl(1.0/0.0)` is not a GCC constant).
fn long_double_rounded(
    args: &[Expr],
    eval_fn: &dyn Fn(&Expr) -> Option<IrConst>,
    f: fn(f64) -> f64,
) -> Option<IrConst> {
    let x = first_f64(args, eval_fn)?;
    let r = f(x);
    (x.is_finite() && r.is_finite()).then(|| IrConst::long_double(r))
}

/// [`long_double_rounded`] for two arguments.
fn long_double_rounded_2(
    args: &[Expr],
    eval_fn: &dyn Fn(&Expr) -> Option<IrConst>,
    f: fn(f64, f64) -> f64,
) -> Option<IrConst> {
    let (a, b) = (first_f64(args, eval_fn)?, first_f64(&args[1..], eval_fn)?);
    let r = f(a, b);
    (a.is_finite() && b.is_finite() && r.is_finite()).then(|| IrConst::long_double(r))
}

/// Whether a non-finite value was *produced* by arithmetic rather than written
/// down.
///
/// GCC marks the result of an operation that returns a non-finite value from
/// finite operands (`1.0/0.0`, `1e308*10.0`, `0.0/0.0`) as overflowing, and
/// refuses to fold `signbit`, `copysign`, `floor`, `ceil` and `trunc` of such a
/// value, while folding the same value written as a literal (`1e309`) or
/// produced by a builtin (`__builtin_inf()`, `__builtin_nan("")`); an exact
/// operation that only carries an infinity through
/// (`__builtin_inf() + 0.0`, `__builtin_inf() * 1.0`) does not mark it either.
/// All of those rows are pinned in tests/regression/file_scope_init/values.c.
///
/// This walks the expression for an operation with finite operands whose own
/// value is not finite -- exactly the operation GCC would flag.
fn nonfinite_from_arithmetic(expr: &Expr, eval_fn: &dyn Fn(&Expr) -> Option<IrConst>) -> bool {
    let value = |e: &Expr| eval_fn(e).and_then(|v| v.to_f64());
    match expr {
        Expr::BinaryOp(op, lhs, rhs, _)
            if matches!(op, BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div) =>
        {
            let finite = |e: &Expr| value(e).is_some_and(f64::is_finite);
            let nonfinite = |e: &Expr| value(e).is_some_and(|v| !v.is_finite());
            (finite(lhs) && finite(rhs) && nonfinite(expr))
                || nonfinite_from_arithmetic(lhs, eval_fn)
                || nonfinite_from_arithmetic(rhs, eval_fn)
        }
        Expr::Cast(_, inner, _, _) | Expr::UnaryOp(_, inner, _) => {
            nonfinite_from_arithmetic(inner, eval_fn)
        }
        Expr::Conditional(_, then_e, else_e, _) => {
            nonfinite_from_arithmetic(then_e, eval_fn) || nonfinite_from_arithmetic(else_e, eval_fn)
        }
        _ => false,
    }
}

/// An integral `f64` operation (`floor`, `ceil`, `trunc`): exact for every
/// value it is given, so it folds for a non-finite value that a literal or a
/// builtin wrote down, and declines one that arithmetic produced.
fn integral_f64(
    args: &[Expr],
    eval_fn: &dyn Fn(&Expr) -> Option<IrConst>,
    f: fn(f64) -> f64,
) -> Option<IrConst> {
    let arg = args.first()?;
    let x = first_f64(args, eval_fn)?;
    if !x.is_finite() && nonfinite_from_arithmetic(arg, eval_fn) {
        return None;
    }
    Some(IrConst::F64(f(x)))
}

/// [`integral_f64`] in `float` precision.
fn integral_f32(
    args: &[Expr],
    eval_fn: &dyn Fn(&Expr) -> Option<IrConst>,
    f: fn(f32) -> f32,
) -> Option<IrConst> {
    let arg = args.first()?;
    let x = first_f32(args, eval_fn)?;
    if !x.is_finite() && nonfinite_from_arithmetic(arg, eval_fn) {
        return None;
    }
    Some(IrConst::F32(f(x)))
}

/// [`integral_f64`] with a `long double` carrier.
fn integral_long_double(
    args: &[Expr],
    eval_fn: &dyn Fn(&Expr) -> Option<IrConst>,
    f: fn(f64) -> f64,
) -> Option<IrConst> {
    let arg = args.first()?;
    let x = first_f64(args, eval_fn)?;
    if !x.is_finite() && nonfinite_from_arithmetic(arg, eval_fn) {
        return None;
    }
    Some(IrConst::long_double(f(x)))
}

/// `copysign`: the magnitude of the first argument with the sign of the second,
/// folded for every value except an arithmetic-produced non-finite one.
fn copysign_f64(args: &[Expr], eval_fn: &dyn Fn(&Expr) -> Option<IrConst>) -> Option<IrConst> {
    let (magnitude, sign) = (first_f64(args, eval_fn)?, first_f64(&args[1..], eval_fn)?);
    if (!magnitude.is_finite() && nonfinite_from_arithmetic(&args[0], eval_fn))
        || (!sign.is_finite() && nonfinite_from_arithmetic(&args[1], eval_fn))
    {
        return None;
    }
    Some(IrConst::F64(magnitude.copysign(sign)))
}

/// [`copysign_f64`] in `float` precision.
fn copysign_f32(args: &[Expr], eval_fn: &dyn Fn(&Expr) -> Option<IrConst>) -> Option<IrConst> {
    let (magnitude, sign) = (first_f32(args, eval_fn)?, first_f32(&args[1..], eval_fn)?);
    if (!magnitude.is_finite() && nonfinite_from_arithmetic(&args[0], eval_fn))
        || (!sign.is_finite() && nonfinite_from_arithmetic(&args[1], eval_fn))
    {
        return None;
    }
    Some(IrConst::F32(magnitude.copysign(sign)))
}

/// [`copysign_f64`] with a `long double` carrier.
fn copysign_long_double(
    args: &[Expr],
    eval_fn: &dyn Fn(&Expr) -> Option<IrConst>,
) -> Option<IrConst> {
    let (magnitude, sign) = (first_f64(args, eval_fn)?, first_f64(&args[1..], eval_fn)?);
    if (!magnitude.is_finite() && nonfinite_from_arithmetic(&args[0], eval_fn))
        || (!sign.is_finite() && nonfinite_from_arithmetic(&args[1], eval_fn))
    {
        return None;
    }
    Some(IrConst::long_double(magnitude.copysign(sign)))
}

/// An integer-valued conversion (`lround`, `lrint`): finite argument, finite
/// result, and a result the carrier can hold exactly -- a value outside
/// `i64` is not folded, because C leaves the conversion undefined there and
/// GCC refuses it too.
fn rounded_i64(
    args: &[Expr],
    eval_fn: &dyn Fn(&Expr) -> Option<IrConst>,
    f: fn(f64) -> f64,
) -> Option<IrConst> {
    let x = first_f64(args, eval_fn)?;
    let r = f(x);
    (x.is_finite() && r.is_finite() && r.abs() < 9.223_372_036_854_776e18)
        .then(|| IrConst::I64(r as i64))
}

/// [`rounded_i64`] with a `float` argument and result.
fn rounded_i64_f32(
    args: &[Expr],
    eval_fn: &dyn Fn(&Expr) -> Option<IrConst>,
    f: fn(f32) -> f32,
) -> Option<IrConst> {
    let x = first_f32(args, eval_fn)?;
    let r = f(x);
    (x.is_finite() && r.is_finite() && r.abs() < 9.223_372e18).then(|| IrConst::I64(r as i64))
}

/// A classification builtin (`isnan`, `isinf`, `isfinite`, `isnormal`): an
/// `int` answer that exists for every argument, non-finite ones included.
fn classified_f64(
    args: &[Expr],
    eval_fn: &dyn Fn(&Expr) -> Option<IrConst>,
    f: fn(f64) -> bool,
) -> Option<IrConst> {
    Some(IrConst::I32(f(first_f64(args, eval_fn)?) as i32))
}

/// Promote a sub-int IrConst (I8/I16) to I32 for unary arithmetic.
///
/// C11 6.3.1.1: Integer promotion converts unsigned char/short to int by
/// zero-extending, and signed char/short by sign-extending. Without this,
/// negate_const(I16(-1)) for unsigned short 65535 would compute -(-1) = 1
/// instead of -(65535) = -65535.
///
/// `is_unsigned` indicates whether the expression has an unsigned type.
#[inline]
/// Evaluate a compile-time builtin call under an explicit builtin policy.
/// `allow_nonprefixed_builtins=false` (`-fno-builtin` / `-ffreestanding`)
/// withdraws EVERY non-`__`-prefixed spelling — the whole bare-name fold
pub fn promote_sub_int(val: IrConst, is_unsigned: bool) -> IrConst {
    match &val {
        IrConst::I8(v) => {
            if is_unsigned {
                IrConst::I32(*v as u8 as i32)
            } else {
                IrConst::I32(*v as i32)
            }
        }
        IrConst::I16(v) => {
            if is_unsigned {
                IrConst::I32(*v as u16 as i32)
            } else {
                IrConst::I32(*v as i32)
            }
        }
        _ => val,
    }
}

/// Evaluate a constant expression as raw u64 bits, with fallback to value conversion.
///
/// Used by `eval_const_expr_as_bits` for non-cast expressions. Converts the
/// IrConst result to its raw bit representation.
#[inline]
pub fn irconst_to_bits(val: &IrConst) -> (u64, bool) {
    let bits = match val {
        IrConst::F32(v) => *v as i64 as u64,
        IrConst::F64(v) => *v as i64 as u64,
        _ => val.to_i64().unwrap_or(0) as u64,
    };
    (bits, true) // default to signed
}

/// C11 6.3.1.1p2 integer promotion on a (byte size, is_unsigned) pair.
///
/// Types with rank below `int` promote to `int` when `int` can represent all
/// their values, which holds for every sub-`int` integer type in the LP64 /
/// ILP32 models LCCC targets (`unsigned short` is 16 bits < 32). The promoted
/// type is therefore always **signed** `int`; only types at least as wide as
/// `int` keep their own signedness.
#[inline]
pub fn integer_promote_size_sign(size: usize, is_unsigned: bool) -> (usize, bool) {
    if size < 4 {
        (4, false)
    } else {
        (size, is_unsigned)
    }
}

/// Evaluate a binary operation on constant operands with given type parameters.
///
/// This wraps `const_arith::eval_const_binop` with the C usual arithmetic
/// conversion logic (C11 6.3.1.8). For shifts, only the LHS type determines
/// the result type (C11 6.5.7); for other ops, use the wider of both types.
///
/// Parameters:
/// - `lhs_size`: byte size of the LHS operand's *unpromoted* C type
/// - `lhs_unsigned`: whether that unpromoted type is unsigned
/// - `rhs_size`: byte size of the RHS operand's *unpromoted* C type
/// - `rhs_unsigned`: whether that unpromoted type is unsigned
///
/// Integer promotion (C11 6.3.1.1p2) is applied HERE, not by the callers:
/// every type narrower than `int` (`_Bool`, `char`, `short`, and their
/// unsigned variants, whose full value range fits in `int`) promotes to
/// **signed** `int`. Callers used to clamp the size with `.max(4)` while
/// forwarding the unpromoted signedness, so `(uint8_t)201 | (int16_t)-1`
/// folded as `unsigned int` (0xFFFFFFFF, then zero-extended to 64 bits)
/// instead of `int` -1 (stress lab: intexpr seed 5 case 22, wrong at -O0).
pub fn eval_binop_with_types(
    op: &BinOp,
    lhs: &IrConst,
    rhs: &IrConst,
    lhs_size: usize,
    lhs_unsigned: bool,
    rhs_size: usize,
    rhs_unsigned: bool,
) -> Option<IrConst> {
    let (lhs_size, lhs_unsigned) = integer_promote_size_sign(lhs_size, lhs_unsigned);
    let (rhs_size, rhs_unsigned) = integer_promote_size_sign(rhs_size, rhs_unsigned);
    let is_shift = matches!(op, BinOp::Shl | BinOp::Shr);

    // For shifts (C11 6.5.7): result type is the promoted LHS type only.
    // For other ops: apply C's usual arithmetic conversions using both operand types.
    let (is_32bit, is_unsigned) = if is_shift {
        (lhs_size <= 4, lhs_unsigned)
    } else {
        let result_size = lhs_size.max(rhs_size);
        let is_unsigned = if lhs_size == rhs_size {
            lhs_unsigned || rhs_unsigned
        } else if lhs_size > rhs_size {
            lhs_unsigned
        } else {
            rhs_unsigned
        };
        (result_size <= 4, is_unsigned)
    };
    const_arith::eval_const_binop(
        op,
        lhs,
        rhs,
        is_32bit,
        is_unsigned,
        lhs_unsigned,
        rhs_unsigned,
    )
}

// ---------------------------------------------------------------------------
// Value semantics of the 16-byte carrier
//
// `IrConst::I128` carries three different C types: `__int128` (two's
// complement), `_Float128` (IEEE binary128 bits) and `_Decimal128` (BID128
// bits). Only the C type tells them apart, so every truth test, conversion and
// fold of such a constant must consult it; the bits alone misread -0.0Q and
// -0.0DL as nonzero and `2.75Q` as a huge integer. Sema and lowering both fold
// these constants, and both use the helpers below so the two layers cannot
// disagree.
// ---------------------------------------------------------------------------

/// The C truth value (`!= 0`, C11 6.3.1.2) of a folded constant of type `ct`.
/// Binary floating values compare by value (+-0 false, NaN true); decimal
/// values by BID decoding (every zero quantum false). `ct == None` falls back
/// to the carrier's own notion, which is only correct for unambiguous kinds.
pub fn const_truth(val: &IrConst, ct: Option<&CType>) -> bool {
    use super::decimal;
    match (ct, val) {
        (Some(CType::Float128), IrConst::I128(bits)) => (*bits as u128) & !(1u128 << 127) != 0,
        (Some(CType::Decimal128), IrConst::I128(bits)) => {
            let b = *bits as u128;
            !decimal::bid128_is_zero((b >> 64) as u64, b as u64)
        }
        (Some(CType::Decimal32), IrConst::I32(v)) => !decimal::bid32_is_zero(*v as u32),
        (Some(CType::Decimal64), IrConst::I64(v)) => !decimal::bid64_is_zero(*v as u64),
        _ => val.is_nonzero(),
    }
}

/// The exact binary128 value of a real constant `val` of C type `ct`, or
/// `None` for non-arithmetic types. Integers up to 64 bits and every binary
/// floating constant are exact; 128-bit integers and decimal values round
/// once (nearest-even), exactly like the runtime conversions
/// (`__floattitf`, `__bid_*totf`). A long double constant contributes its
/// value in the target format (x87 bytes are rounded to 64 bits first).
pub fn f128_bytes_of(val: &IrConst, ct: Option<&CType>) -> Option<[u8; 16]> {
    use super::decimal;
    use super::long_double as ld;
    let ct = ct?;
    let bid = |neg: bool, dec: Option<(u128, i32)>, special_bits_nan: bool| -> [u8; 16] {
        match dec {
            Some((c, e)) => {
                let sign = if neg { "-" } else { "" };
                ld::parse_long_double_to_f128_bytes(&format!("{sign}{c}e{e}"))
            }
            None if special_bits_nan => ld::parse_long_double_to_f128_bytes("nan"),
            None => ld::parse_long_double_to_f128_bytes(if neg { "-inf" } else { "inf" }),
        }
    };
    match ct {
        CType::Float128 => match val {
            IrConst::I128(bits) => Some((*bits as u128).to_le_bytes()),
            IrConst::Zero => Some([0; 16]),
            _ => None,
        },
        CType::Decimal32 | CType::Decimal64 | CType::Decimal128 => {
            // Sign, finite (coefficient, exponent), or NaN-vs-infinity from
            // the combination field (0b11111 = NaN, 0b11110 = infinity).
            let (neg, dec, nan) = match (ct, val) {
                (CType::Decimal32, IrConst::D32(v)) => {
                    let d = decimal::decode_bid32(*v);
                    (
                        v >> 31 != 0,
                        d.map(|(_, c, e)| (u128::from(c), e)),
                        (v >> 26) & 0x1F == 0x1F,
                    )
                }
                (CType::Decimal64, IrConst::D64(v)) => {
                    let d = decimal::decode_bid64(*v);
                    (
                        v >> 63 != 0,
                        d.map(|(_, c, e)| (u128::from(c), e)),
                        (v >> 58) & 0x1F == 0x1F,
                    )
                }
                (CType::Decimal128, IrConst::I128(v)) => {
                    let (hi, lo) = ((*v as u128 >> 64) as u64, *v as u64);
                    let d = decimal::decode_bid128(hi, lo);
                    (
                        hi >> 63 != 0,
                        d.map(|(_, c, e)| (c, e)),
                        (hi >> 58) & 0x1F == 0x1F,
                    )
                }
                _ => return None,
            };
            Some(bid(neg, dec, nan))
        }
        _ if !(ct.is_integer() || ct.is_floating()) => None,
        _ => Some(match val {
            IrConst::LongDouble(_, bytes) => ld::long_double_value_bytes(bytes),
            IrConst::F64(f) => ld::f64_to_f128_bytes_lossless(*f),
            IrConst::F32(f) => ld::f64_to_f128_bytes_lossless(f64::from(*f)),
            IrConst::I128(v) if ct.is_unsigned() => ld::u128_to_f128_bytes(*v as u128),
            IrConst::I128(v) => ld::i128_to_f128_bytes(*v),
            IrConst::D32(_) | IrConst::D64(_) => return None,
            other => {
                let v = other.to_i64()?;
                if ct.is_unsigned() {
                    ld::u64_to_f128_bytes(v as u64)
                } else {
                    ld::i64_to_f128_bytes(v)
                }
            }
        }),
    }
}

/// `l op r` in binary128 with IEEE semantics, exactly rounded (the soft-float
/// helpers match libgcc's `__addtf3` & co. in round-to-nearest). Comparisons
/// with a NaN are false except `!=`, and +0 == -0. Results are the carrier
/// for arithmetic and an `int` 0/1 for comparisons and logical operators.
pub fn fold_f128_binop(op: &BinOp, l: &[u8; 16], r: &[u8; 16]) -> Option<IrConst> {
    use super::long_double as ld;
    let carrier = |b: [u8; 16]| IrConst::I128(u128::from_le_bytes(b) as i128);
    let truth = |b: bool| IrConst::I32(i32::from(b));
    let nonzero = |b: &[u8; 16]| u128::from_le_bytes(*b) & !(1u128 << 127) != 0;
    let ord = ld::f128_cmp(l, r);
    let ordered = ord != i32::MIN;
    Some(match op {
        BinOp::Add => carrier(ld::f128_add(l, r)),
        BinOp::Sub => carrier(ld::f128_sub(l, r)),
        BinOp::Mul => carrier(ld::f128_mul(l, r)),
        BinOp::Div => carrier(ld::f128_div(l, r)),
        BinOp::Lt => truth(ordered && ord < 0),
        BinOp::Le => truth(ordered && ord <= 0),
        BinOp::Gt => truth(ordered && ord > 0),
        BinOp::Ge => truth(ordered && ord >= 0),
        BinOp::Eq => truth(ordered && ord == 0),
        BinOp::Ne => truth(!(ordered && ord == 0)),
        BinOp::LogicalAnd => truth(nonzero(l) && nonzero(r)),
        BinOp::LogicalOr => truth(nonzero(l) || nonzero(r)),
        _ => return None,
    })
}

/// Convert the binary128 value `bytes` to the real C type `target` by value
/// (C11 6.3.1.4, 6.3.1.5), as the runtime conversions do in round-to-nearest:
/// one rounding to `float`, `double` or an x87 `long double`; truncation
/// toward zero to integers. An out-of-range integer conversion is undefined;
/// it folds like GCC's constant folder (NaN -> 0, saturation at the type's
/// bounds) so an initializer stays constant. Integer results follow the
/// sema convention: unsigned sub-int values are zero-extended `I32`.
/// Decimal and non-arithmetic targets return `None`.
pub fn f128_to_ctype(bytes: &[u8; 16], target: &CType) -> Option<IrConst> {
    use super::long_double as ld;
    let raw = u128::from_le_bytes(*bytes);
    Some(match target {
        CType::Float128 => IrConst::I128(raw as i128),
        CType::Enum(e) => return f128_to_ctype(bytes, &e.compatible_ctype()),
        CType::LongDouble => {
            let v = ld::long_double_value_bytes(bytes);
            IrConst::long_double_with_bytes(ld::f128_bytes_to_f64(&v), v)
        }
        CType::Double => IrConst::F64(ld::f128_bytes_to_f64(bytes)),
        CType::Float => IrConst::F32(ld::f128_bytes_to_f32(bytes)),
        CType::Bool => IrConst::I8(i8::from(raw & !(1u128 << 127) != 0)),
        t if t.is_integer() => {
            let size = t.size();
            let unsigned = t.is_unsigned();
            let bits = (size * 8) as u32;
            let is_nan = (raw >> 112) & 0x7FFF == 0x7FFF && raw & ((1u128 << 112) - 1) != 0;
            let negative = raw >> 127 != 0;
            // Saturated two's-complement image, truncated toward zero.
            let v: u128 = if is_nan {
                0
            } else if unsigned {
                let max = if bits == 128 {
                    u128::MAX
                } else {
                    (1u128 << bits) - 1
                };
                match ld::f128_bytes_to_i128(bytes) {
                    Some(v) if v < 0 => 0,
                    _ if negative => 0,
                    _ => ld::f128_bytes_to_u128(bytes).map_or(max, |u| u.min(max)),
                }
            } else {
                let max = (1i128 << (bits - 1)).wrapping_sub(1);
                let min = if bits == 128 {
                    i128::MIN
                } else {
                    -(1i128 << (bits - 1))
                };
                let v = ld::f128_bytes_to_i128(bytes).unwrap_or(if negative { min } else { max });
                v.clamp(min, max) as u128
            };
            match (size, unsigned) {
                (1, false) => IrConst::I8(v as i8),
                (2, false) => IrConst::I16(v as i16),
                (1 | 2 | 4, _) => IrConst::I32(v as u32 as i32),
                (8, _) => IrConst::I64(v as u64 as i64),
                (16, _) => IrConst::I128(v as i128),
                _ => return None,
            }
        }
        _ => return None,
    })
}

#[cfg(test)]
mod string_length_tests {
    use super::{eval_builtin_call, eval_builtin_call_with_mode, narrow_string_strlen};
    use crate::common::source::Span;
    use crate::frontend::parser::ast::Expr;
    use crate::ir::constants::IrConst;

    #[test]
    fn narrow_strlen_stops_at_first_embedded_nul() {
        assert_eq!(narrow_string_strlen("ab\0cd"), 2);
        let literal = || Expr::StringLiteral("ab\0cd".to_string(), Span::dummy());
        assert_eq!(
            eval_builtin_call("strlen", &[literal()], &|_| None),
            Some(IrConst::I64(2))
        );
        // Policy interaction: `-fno-builtin` keeps the ordinary spelling a
        // call (None here), while the explicit __builtin spelling still
        // folds — GCC-compatible, at every tier.
        assert_eq!(
            eval_builtin_call_with_mode("strlen", &[literal()], &|_| None, false),
            None
        );
        assert_eq!(
            eval_builtin_call_with_mode("__builtin_strlen", &[literal()], &|_| None, false),
            Some(IrConst::I64(2))
        );
    }

    #[test]
    fn narrow_strlen_uses_existing_character_count_for_plain_literals() {
        assert_eq!(narrow_string_strlen("abcd"), 4);
    }
}
