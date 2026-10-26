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
        Expr::FloatLiteralHalf(_, bits, _) => Some(IrConst::I16(*bits as i16)),
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

/// The sign of the first difference between two narrow string literals compared
/// as `unsigned char`: -1, 0 or 1, or `None` when the answer is not foldable.
///
/// `limit` is `memcmp`'s `n`; `None` means `strcmp`, i.e. stop at the first NUL.
/// Both answer with the SIGN, which is all C specifies and what gcc 14.2 and
/// clang 23.1.3 both emit for a folded literal compare (measured: `memcmp("c",
/// "a", 1)` -> 1, `memcmp("z", "a", 1)` -> 1, `memcmp("\x80", "\x7f", 1)` -> 1,
/// `strcmp("abc", "abx")` -> -1). glibc's runtime answer is a magnitude (25, 1,
/// -21), so folding to the magnitude would make the same program print different
/// numbers at -O0 and -O2.
///
/// `strncmp` is deliberately absent from the table: gcc folds it to the glibc
/// magnitude (2) and clang to the sign (1), so there is no vendor consensus to
/// fold towards and a unit test pins the refusal.
pub(crate) fn narrow_compare_sign(a: &str, b: &str, limit: Option<usize>) -> Option<i32> {
    let n = match limit {
        Some(n) => {
            // Fold only inside the bytes both literals actually spell: the
            // terminating NUL is part of the literal's array object but not of
            // the stored text, and reasoning about reading it buys nothing --
            // declining leaves the call, which answers the same sign.
            if n > a.chars().count() || n > b.chars().count() {
                return None;
            }
            n
        }
        None => usize::MAX,
    };
    let mut left = a.chars();
    let mut right = b.chars();
    for _ in 0..n {
        // Narrow literals are stored one C byte per Rust char (see lex_string),
        // so a char IS the byte; `str::len()` would be the UTF-8 encoding size.
        let (l, r) = match (left.next(), right.next()) {
            (Some(l), Some(r)) => (l as u32 as u8, r as u32 as u8),
            // `strcmp`: the stored text ends where the terminator begins, and
            // the terminator is 0 and part of the object.
            (None, None) => return Some(0),
            (None, Some(_)) => return Some(-1),
            (Some(_), None) => return Some(1),
        };
        if l != r {
            return Some(if l > r { 1 } else { -1 });
        }
        if limit.is_none() && l == 0 {
            return Some(0);
        }
    }
    Some(0)
}

/// The two narrow string literal arguments of a `strcmp`/`memcmp` call.
///
/// Wide and char16 literals are separate AST variants (`WideStringLiteral`,
/// `Char16StringLiteral`) and separate IR vectors, so matching `StringLiteral`
/// alone cannot pick one up -- which is what makes the `as u8` byte view in
/// `narrow_compare_sign` exact rather than a truncation.
fn two_narrow_literals(args: &[Expr]) -> Option<(&str, &str)> {
    let (first, second) = (args.first()?, args.get(1)?);
    let (Expr::StringLiteral(a, _), Expr::StringLiteral(b, _)) = (first, second) else {
        return None;
    };
    Some((a.as_str(), b.as_str()))
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

/// `abs`/`labs`/`llabs` of an integer constant, in `int` or (`wide`) a
/// 64-bit type; `None` for the most negative value, whose absolute value
/// overflows.
fn int_abs(
    args: &[Expr],
    eval_fn: &dyn Fn(&Expr) -> Option<IrConst>,
    wide: bool,
) -> Option<IrConst> {
    let v = eval_fn(args.first()?)?.to_i64()?;
    if wide {
        v.checked_abs().map(IrConst::I64)
    } else {
        (v as i32).checked_abs().map(IrConst::I32)
    }
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
        "__builtin_strcmp" | "strcmp" => {
            let (a, b) = two_narrow_literals(args)?;
            Some(IrConst::I32(narrow_compare_sign(a, b, None)?))
        }
        "__builtin_memcmp" | "memcmp" => {
            let (a, b) = two_narrow_literals(args)?;
            // The length has to be a compile-time value in range: a negative
            // length is UB and one past either literal is out of the stored
            // text, and both decline into the call rather than guess.
            let n = eval_fn(args.get(2)?)?.to_i64()?;
            if n < 0 {
                return None;
            }
            Some(IrConst::I32(narrow_compare_sign(a, b, Some(n as usize))?))
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
        // The absolute value in the function's own type; the most negative
        // value has none (undefined), so it does not fold.
        "__builtin_abs" | "abs" => int_abs(args, eval_fn, false),
        "__builtin_labs" | "labs" => {
            int_abs(args, eval_fn, !crate::common::types::target_is_32bit())
        }
        "__builtin_llabs" | "llabs" | "__builtin_imaxabs" | "imaxabs" => {
            int_abs(args, eval_fn, true)
        }
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
        // A long double NaN keeps its payload in the target format (x87:
        // `nanl("7")` is 0x7fff:c000000000000007; binary128: low significand
        // bits). The constant's binary128 bytes carry it losslessly: the
        // x87<->binary128 NaN mappings are top-aligned and inverse.
        "__builtin_nanl" | "nanl" => Some(nan_long_double(nan_payload(args)?, false)),
        "__builtin_nansl" | "nansl" => Some(nan_long_double(nan_payload(args)?, true)),
        // _Float128 (and GCC's x86 `q` spellings for __float128): the
        // binary128 bits ride the I128 carrier.
        "__builtin_nanf128" | "__builtin_nanq" | "nanf128" => {
            Some(nan_f128(nan_payload(args)?, false))
        }
        "__builtin_nansf128" | "__builtin_nansq" => Some(nan_f128(nan_payload(args)?, true)),
        "__builtin_inff128"
        | "__builtin_infq"
        | "__builtin_huge_valf128"
        | "__builtin_huge_valq" => Some(IrConst::I128((0x7FFFu128 << 112) as i128)),
        "__builtin_inff16" | "__builtin_huge_valf16" => Some(IrConst::I16(0x7c00u16 as i16)),
        "__builtin_nanf16" | "nanf16" => {
            let sig = nan_significand(nan_payload(args)?, false, 10) as u16;
            Some(IrConst::I16((0x7c00u16 | sig) as i16))
        }
        "__builtin_nansf16" => {
            let sig = nan_significand(nan_payload(args)?, true, 10) as u16;
            Some(IrConst::I16((0x7c00u16 | sig) as i16))
        }
        "__builtin_nansf16b" => {
            let sig = nan_significand(nan_payload(args)?, true, 7) as u16;
            Some(IrConst::I16((0x7f80u16 | sig) as i16))
        }
        _ => eval_half_builtin_call(name, args, eval_fn),
    }
}

/// Evaluate type-generic floating-point classification builtins (`isnan`,
/// `isinf`, `isinf_sign`, `isfinite`, `isnormal`, `issignaling`, `signbit`,
/// `fpclassify`) directly on the exact 16-bit IEEE `_Float16` / `__bf16`
/// representation so subnormals (`0x1p-15f16`) and signaling NaNs are never
/// distorted by widening to `f32`/`f64`.
pub fn eval_half_fpclassify_builtin(
    name: &str,
    args: &[Expr],
    arg_ct: &CType,
    eval_fn: &dyn Fn(&Expr) -> Option<IrConst>,
) -> Option<IrConst> {
    let target_arg = if name == "__builtin_fpclassify" {
        args.get(5)?
    } else {
        args.first()?
    };
    let bits = half_bits_of(&eval_fn(target_arg)?)?;
    let (exp_mask, frac_mask, quiet_bit): (u16, u16, u16) = match arg_ct {
        CType::Float16 => (0x7c00, 0x03ff, 0x0200),
        CType::BFloat16 => (0x7f80, 0x007f, 0x0040),
        _ => return None,
    };
    let abs = bits & 0x7fff;
    let exp = abs & exp_mask;
    let frac = abs & frac_mask;
    let neg = (bits & 0x8000) != 0;
    let is_nan = exp == exp_mask && frac != 0;
    let is_inf = exp == exp_mask && frac == 0;
    let is_finite = exp != exp_mask;
    let is_normal = exp != 0 && exp != exp_mask;
    let is_subnormal = exp == 0 && frac != 0;
    let is_signaling = is_nan && (frac & quiet_bit) == 0;
    match name {
        "__builtin_isnan" | "isnan" => Some(IrConst::I32(i32::from(is_nan))),
        "__builtin_isinf" | "isinf" => Some(IrConst::I32(i32::from(is_inf))),
        "__builtin_isinf_sign" => Some(IrConst::I32(if !is_inf {
            0
        } else if neg {
            -1
        } else {
            1
        })),
        "__builtin_isfinite" | "isfinite" => Some(IrConst::I32(i32::from(is_finite))),
        "__builtin_isnormal" | "isnormal" => Some(IrConst::I32(i32::from(is_normal))),
        "__builtin_issignaling" => Some(IrConst::I32(i32::from(is_signaling))),
        "__builtin_signbit" | "signbit" => {
            if !is_finite && nonfinite_from_arithmetic(target_arg, eval_fn) {
                return None;
            }
            Some(IrConst::I32(i32::from(neg)))
        }
        "__builtin_fpclassify" if args.len() >= 6 => {
            let idx = if is_nan {
                0
            } else if is_inf {
                1
            } else if is_normal {
                2
            } else if is_subnormal {
                3
            } else {
                4
            };
            eval_fn(&args[idx])
        }
        _ => None,
    }
}

/// Widen a constant `_Float16` / `__bf16` value (`IrConst::I16(bits)`) to
/// `IrConst::F32` before passing it into generic `f32`/`f64` builtin folders
/// (which treat `IrConst::I16` as `short`), while leaving `*f16*` builtins
/// untouched so raw signaling-NaN bits stay intact.
pub fn widen_half_const_for_builtin(
    builtin_name: &str,
    val: IrConst,
    ct: Option<&CType>,
) -> IrConst {
    if builtin_name.ends_with("f16") || builtin_name.ends_with("f16b") {
        return val;
    }
    match ct {
        Some(CType::Float16) => half_bits_of(&val)
            .map(|b| IrConst::F32(super::half_float::f16_bits_to_f32(b)))
            .unwrap_or(val),
        Some(CType::BFloat16) => half_bits_of(&val)
            .map(|b| IrConst::F32(super::half_float::bf16_bits_to_f32(b)))
            .unwrap_or(val),
        _ => val,
    }
}

fn eval_half_builtin_call(
    name: &str,
    args: &[Expr],
    eval_fn: &dyn Fn(&Expr) -> Option<IrConst>,
) -> Option<IrConst> {
    use super::half_float as hf;
    let arg_bits = |idx: usize| -> Option<u16> {
        let e = args.get(idx)?;
        let v = eval_fn(e)?;
        match v {
            IrConst::I16(b) => Some(b as u16),
            IrConst::F32(f) => Some(hf::f32_to_f16_bits(f)),
            IrConst::F64(f) => Some(hf::f64_to_f16_bits(f)),
            IrConst::I32(i) => Some(hf::i128_to_f16_bits(i128::from(i))),
            IrConst::I64(i) => Some(hf::i128_to_f16_bits(i128::from(i))),
            IrConst::Zero => Some(0),
            _ => None,
        }
    };
    let pack = |f: f32| IrConst::I16(hf::f32_to_f16_bits(f) as i16);
    let unary_integral = |op: fn(f32) -> f32| -> Option<IrConst> {
        let f = hf::f16_bits_to_f32(arg_bits(0)?);
        if !f.is_finite() && nonfinite_from_arithmetic(args.first()?, eval_fn) {
            return None;
        }
        Some(pack(op(f)))
    };
    let unary_finite = |op: fn(f32) -> f32| -> Option<IrConst> {
        let f = hf::f16_bits_to_f32(arg_bits(0)?);
        let r = op(f);
        (f.is_finite() && r.is_finite()).then(|| pack(r))
    };
    match name {
        "__builtin_fabsf16" => Some(IrConst::I16((arg_bits(0)? & 0x7fff) as i16)),
        "__builtin_copysignf16" => {
            let x = arg_bits(0)?;
            let y = arg_bits(1)?;
            Some(IrConst::I16(((x & 0x7fff) | (y & 0x8000)) as i16))
        }
        "__builtin_ceilf16" => unary_integral(f32::ceil),
        "__builtin_floorf16" => unary_integral(f32::floor),
        "__builtin_truncf16" => unary_integral(f32::trunc),
        "__builtin_roundf16" => unary_finite(f32::round),
        "__builtin_roundevenf16" => unary_finite(f32::round_ties_even),
        "__builtin_sqrtf16" => unary_finite(f32::sqrt),
        "__builtin_fmaxf16" => {
            let a = hf::f16_bits_to_f32(arg_bits(0)?);
            let b = hf::f16_bits_to_f32(arg_bits(1)?);
            (a.is_finite() && b.is_finite()).then(|| pack(a.max(b)))
        }
        "__builtin_fminf16" => {
            let a = hf::f16_bits_to_f32(arg_bits(0)?);
            let b = hf::f16_bits_to_f32(arg_bits(1)?);
            (a.is_finite() && b.is_finite()).then(|| pack(a.min(b)))
        }
        "__builtin_fmaf16" => {
            let a = hf::f16_bits_to_f32(arg_bits(0)?);
            let b = hf::f16_bits_to_f32(arg_bits(1)?);
            let c = hf::f16_bits_to_f32(arg_bits(2)?);
            let r = (a as f64).mul_add(b as f64, c as f64);
            (a.is_finite() && b.is_finite() && c.is_finite() && r.is_finite())
                .then(|| IrConst::I16(hf::f64_to_f16_bits(r) as i16))
        }
        "__builtin_nextafterf16" => {
            let x = arg_bits(0)?;
            let y = arg_bits(1)?;
            ((x & 0x7fff) < 0x7c00 && (y & 0x7fff) <= 0x7c00)
                .then(|| IrConst::I16(hf::nextafterf16_bits(x, y) as i16))
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

/// The trailing significand of a NaN with `frac_bits` fraction bits carrying
/// `payload`, as GCC encodes it (real.c `encode_ieee_*`, measured against
/// GCC 16.2): a quiet NaN sets the top fraction bit over the payload; a
/// signaling NaN clears it, and when nothing is left (empty payload, or one
/// whose bits all fall outside the field) uses the canonical signaling bit
/// just below it -- never the all-zero fraction, which would be infinity
/// (`nans("0x10000000000000")` is 0x7ff4000000000000).
fn nan_significand(payload: u64, signaling: bool, frac_bits: u32) -> u128 {
    let quiet = 1u128 << (frac_bits - 1);
    let p = u128::from(payload) & ((1u128 << frac_bits) - 1);
    if !signaling {
        return quiet | p;
    }
    match p & !quiet {
        0 => quiet >> 1,
        p => p,
    }
}

/// A `double` NaN carrying `payload`: quiet for `__builtin_nan`, signaling for
/// `__builtin_nans`. See [`nan_significand`].
fn nan_f64(payload: u64, signaling: bool) -> IrConst {
    let frac = nan_significand(payload, signaling, 52) as u64;
    IrConst::F64(f64::from_bits(0x7ff0_0000_0000_0000 | frac))
}

/// [`nan_f64`] in `float` precision.
fn nan_f32(payload: u64, signaling: bool) -> IrConst {
    let frac = nan_significand(payload, signaling, 23) as u32;
    IrConst::F32(f32::from_bits(0x7f80_0000 | frac))
}

/// [`nan_f64`] for `_Float128`: the binary128 bits in the I128 carrier.
fn nan_f128(payload: u64, signaling: bool) -> IrConst {
    IrConst::I128(((0x7FFFu128 << 112) | nan_significand(payload, signaling, 112)) as i128)
}

/// [`nan_f64`] for `long double` in the target format: x87 extended (explicit
/// integer bit, 63 fraction bits) or binary128.
fn nan_long_double(payload: u64, signaling: bool) -> IrConst {
    use super::long_double as ld;
    let bytes = if super::types::target_long_double_is_f128() {
        ((0x7FFFu128 << 112) | nan_significand(payload, signaling, 112)).to_le_bytes()
    } else {
        let sig = (1u128 << 63) | nan_significand(payload, signaling, 63);
        ld::x87_bytes_to_f128_bytes(&((0x7FFFu128 << 64) | sig).to_le_bytes())
    };
    IrConst::long_double_with_bytes(f64::NAN, bytes)
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

/// C11 6.3.1.2: converting any nonzero scalar to `_Bool` yields 1.
///
/// This is a property of the VALUE, not of its low bits, so every caller
/// must hand over the full source bits **before** any truncation to the
/// target width. Truncating first is what folded `(_Bool)256`,
/// `(_Bool)65536u` and `(_Bool)67106816ULL` to 0. Defined once because
/// four folding sites (sema `Cast` evaluation, sema `cast_i128_to_ctype`,
/// and both `eval_const_expr_as_bits`) must agree; the site that is not
/// taught the rule is the next miscompile.
#[inline]
pub fn c_bool_truth(bits: u64) -> bool {
    bits != 0
}

/// 128-bit-source form of [`c_bool_truth`]: the value can have nonzero
/// high bits and an all-zero low half, e.g. `(unsigned __int128)1 << 100`,
/// which converts to `_Bool` 1 just like any other nonzero value.
#[inline]
pub fn c_bool_truth_i128(v: i128) -> bool {
    v != 0
}

/// `_Bool` constant (0 or 1, carried as `int`) from full-width source bits.
#[inline]
pub fn c_bool_const(bits: u64) -> IrConst {
    IrConst::I32(c_bool_truth(bits) as i32)
}

/// [`c_bool_const`] for a source carried in the `__int128` carrier.
#[inline]
pub fn c_bool_const_i128(v: i128) -> IrConst {
    IrConst::I32(c_bool_truth_i128(v) as i32)
}

/// Reduce an integer constant to its operand's width when the value rides
/// a wider carrier.
///
/// The evaluators keep `unsigned int` in the `I64` carrier so the full
/// 32-bit range survives; the price is that a carrier-wide
/// `negate_const`/`bitnot_const` computes mod 2^64 instead of mod 2^32.
/// C11 6.5.3.3p1/p4 give `-x` and `~x` the operand's *promoted* type and
/// 6.3.1.3p2 makes the **unsigned** result modular in that width, so the
/// reduction must happen there -- and only there.
///
/// A signed 32-bit operand also rides `I64` whenever the surrounding
/// expression is 64-bit, so the carrier width alone does NOT prove
/// unsignedness (an earlier version of this function asserted that it
/// did). Masking a signed result zero-extends it: `(size_t)-(enum{3})`
/// folded to `4294967293` instead of `18446744073709551613`. The
/// signedness of the operand is therefore part of the rule, and both
/// callers already compute it for [`promote_sub_int`].
///
/// `operand_size` is the size in bytes of the operand type. Only 4 can
/// matter: a sub-int operand is promoted to `int` before the operator
/// runs, so it arrives in the `I32` carrier and never reaches here, and
/// an unknown size keeps the carrier rule -- the only choice that cannot
/// corrupt a 64-bit operand.
#[inline]
fn reduce_to_operand_width(
    val: IrConst,
    is_unsigned: bool,
    operand_size: Option<usize>,
) -> IrConst {
    match val {
        IrConst::I64(v) if is_unsigned && operand_size == Some(4) => IrConst::I64(v as u32 as i64),
        other => other,
    }
}

/// Unary minus in the operand's promoted type (C11 6.5.3.3p1), with the
/// unsigned result reduced modulo 2^width of that type (6.3.1.3p2).
///
/// `-(3844603080U)` must fold to `450364216`; negating the 64-bit carrier
/// directly produced `18446744069864948536` and flipped a folded `min()`.
#[inline]
pub fn negate_const_promoted(
    val: IrConst,
    is_unsigned: bool,
    operand_size: Option<usize>,
) -> Option<IrConst> {
    let promoted = promote_sub_int(val, is_unsigned);
    let negated = const_arith::negate_const(promoted)?;
    Some(reduce_to_operand_width(negated, is_unsigned, operand_size))
}

/// Bitwise NOT in the operand's promoted type (C11 6.5.3.3p4), with the
/// unsigned result reduced modulo 2^width of that type (6.3.1.3p2).
///
/// Without the reduction `~0xFFFFFFFFu` complements to
/// `0xFFFFFFFF_00000000` instead of 0, which flipped a short-circuit use
/// (`regress_logical_and_const_bitnot_short_circuit`).
#[inline]
pub fn bitnot_const_promoted(
    val: IrConst,
    is_unsigned: bool,
    operand_size: Option<usize>,
) -> Option<IrConst> {
    let promoted = promote_sub_int(val, is_unsigned);
    let inverted = const_arith::bitnot_const(promoted)?;
    Some(reduce_to_operand_width(inverted, is_unsigned, operand_size))
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
        (Some(CType::Float16 | CType::BFloat16), IrConst::I16(bits)) => {
            (*bits as u16) & 0x7fff != 0
        }
        (Some(CType::Float16 | CType::BFloat16), IrConst::I32(bits)) => {
            (*bits as u16) & 0x7fff != 0
        }
        (Some(CType::Float16 | CType::BFloat16), IrConst::I64(bits)) => {
            (*bits as u16) & 0x7fff != 0
        }
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
        CType::Float16 => {
            let bits = half_bits_of(val)?;
            Some(ld::f64_to_f128_bytes_lossless(
                super::half_float::f16_bits_to_f64(bits),
            ))
        }
        CType::BFloat16 => {
            let bits = half_bits_of(val)?;
            Some(ld::f64_to_f128_bytes_lossless(
                super::half_float::bf16_bits_to_f64(bits),
            ))
        }
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
        CType::Float16 => {
            let rto = if ((raw >> 112) & 0x7fff) != 0x7fff && (raw & ((1u128 << 60) - 1)) != 0 {
                (raw & !((1u128 << 60) - 1)) | (1u128 << 60)
            } else {
                raw
            };
            let f = ld::f128_bytes_to_f64(&rto.to_le_bytes());
            IrConst::I16(super::half_float::f64_to_f16_bits(f) as i16)
        }
        CType::BFloat16 => {
            let rto = if ((raw >> 112) & 0x7fff) != 0x7fff && (raw & ((1u128 << 60) - 1)) != 0 {
                (raw & !((1u128 << 60) - 1)) | (1u128 << 60)
            } else {
                raw
            };
            let f = ld::f128_bytes_to_f64(&rto.to_le_bytes());
            IrConst::I16(super::half_float::f64_to_bf16_bits(f) as i16)
        }
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

/// Extract the raw 16-bit IEEE carrier image of a `_Float16` / `__bf16` constant.
pub fn half_bits_of(val: &IrConst) -> Option<u16> {
    match val {
        IrConst::I16(v) => Some(*v as u16),
        IrConst::I32(v) => Some(*v as u16),
        IrConst::I64(v) => Some(*v as u16),
        IrConst::Zero => Some(0),
        _ => None,
    }
}

/// Value-preserving constant conversion where either `src_ct` or `dst_ct` is
/// `_Float16` or `__bf16`.
pub fn cast_half_const(val: &IrConst, src_ct: Option<&CType>, dst_ct: &CType) -> Option<IrConst> {
    use super::half_float as hf;
    if let CType::Enum(e) = dst_ct {
        return cast_half_const(val, src_ct, &e.compatible_ctype());
    }
    if dst_ct.is_half_float() {
        let is_bf16 = *dst_ct == CType::BFloat16;
        let to_half = |f: f64| -> IrConst {
            let bits = if is_bf16 {
                hf::f64_to_bf16_bits(f)
            } else {
                hf::f64_to_f16_bits(f)
            };
            IrConst::I16(bits as i16)
        };
        if let Some(sct) = src_ct {
            match sct {
                CType::Float16 => {
                    let b = half_bits_of(val)?;
                    return Some(if is_bf16 {
                        IrConst::I16(hf::f32_to_bf16_bits(hf::f16_bits_to_f32(b)) as i16)
                    } else {
                        IrConst::I16(b as i16)
                    });
                }
                CType::BFloat16 => {
                    let b = half_bits_of(val)?;
                    return Some(if is_bf16 {
                        IrConst::I16(b as i16)
                    } else {
                        IrConst::I16(hf::f32_to_f16_bits(hf::bf16_bits_to_f32(b)) as i16)
                    });
                }
                CType::Float128 => {
                    let bytes = f128_bytes_of(val, Some(sct))?;
                    return f128_to_ctype(&bytes, dst_ct);
                }
                t if t.is_decimal() => {
                    let bytes = f128_bytes_of(val, Some(sct))?;
                    return f128_to_ctype(&bytes, dst_ct);
                }
                CType::Int128 => {
                    let IrConst::I128(v) = val else { return None };
                    let bits = if is_bf16 {
                        hf::i128_to_bf16_bits(*v)
                    } else {
                        hf::i128_to_f16_bits(*v)
                    };
                    return Some(IrConst::I16(bits as i16));
                }
                CType::UInt128 => {
                    let IrConst::I128(v) = val else { return None };
                    let bits = if is_bf16 {
                        hf::u128_to_bf16_bits(*v as u128)
                    } else {
                        hf::u128_to_f16_bits(*v as u128)
                    };
                    return Some(IrConst::I16(bits as i16));
                }
                _ => {}
            }
        }
        return match val {
            IrConst::F32(f) => Some(IrConst::I16(if is_bf16 {
                hf::f32_to_bf16_bits(*f) as i16
            } else {
                hf::f32_to_f16_bits(*f) as i16
            })),
            IrConst::F64(f) => Some(to_half(*f)),
            IrConst::LongDouble(_, bytes) => {
                let v = super::long_double::long_double_value_bytes(bytes);
                f128_to_ctype(&v, dst_ct)
            }
            IrConst::Zero => Some(IrConst::I16(0)),
            IrConst::I8(v) => {
                let i = if src_ct.is_some_and(CType::is_unsigned) {
                    i128::from(*v as u8)
                } else {
                    i128::from(*v)
                };
                Some(IrConst::I16(if is_bf16 {
                    hf::i128_to_bf16_bits(i) as i16
                } else {
                    hf::i128_to_f16_bits(i) as i16
                }))
            }
            IrConst::I16(v) => {
                let i = if src_ct.is_some_and(CType::is_unsigned) {
                    i128::from(*v as u16)
                } else {
                    i128::from(*v)
                };
                Some(IrConst::I16(if is_bf16 {
                    hf::i128_to_bf16_bits(i) as i16
                } else {
                    hf::i128_to_f16_bits(i) as i16
                }))
            }
            IrConst::I32(v) => {
                let i = if src_ct.is_some_and(CType::is_unsigned) {
                    i128::from(*v as u32)
                } else {
                    i128::from(*v)
                };
                Some(IrConst::I16(if is_bf16 {
                    hf::i128_to_bf16_bits(i) as i16
                } else {
                    hf::i128_to_f16_bits(i) as i16
                }))
            }
            IrConst::I64(v) => {
                if src_ct.is_some_and(CType::is_unsigned) {
                    let u = u128::from(*v as u64);
                    Some(IrConst::I16(if is_bf16 {
                        hf::u128_to_bf16_bits(u) as i16
                    } else {
                        hf::u128_to_f16_bits(u) as i16
                    }))
                } else {
                    let i = i128::from(*v);
                    Some(IrConst::I16(if is_bf16 {
                        hf::i128_to_bf16_bits(i) as i16
                    } else {
                        hf::i128_to_f16_bits(i) as i16
                    }))
                }
            }
            _ => None,
        };
    }

    let sct = src_ct?;
    if !sct.is_half_float() {
        return None;
    }
    let bits = half_bits_of(val)?;
    let f64_val = if *sct == CType::BFloat16 {
        hf::bf16_bits_to_f64(bits)
    } else {
        hf::f16_bits_to_f64(bits)
    };
    let bytes = super::long_double::f64_to_f128_bytes_lossless(f64_val);
    f128_to_ctype(&bytes, dst_ct)
}

/// Constant-fold a binary operation whose common real type is `_Float16` or
/// `__bf16` (or a comparison/logical operation with a half-float operand and
/// no wider floating operand).
pub fn fold_half_binop(
    op: &BinOp,
    lv: &IrConst,
    l_ct: Option<&CType>,
    rv: &IrConst,
    r_ct: Option<&CType>,
) -> Option<IrConst> {
    use super::half_float as hf;
    let lct = l_ct?;
    let rct = r_ct?;
    if !lct.is_arithmetic() || !rct.is_arithmetic() || lct.is_complex() || rct.is_complex() {
        return None;
    }
    let common = CType::usual_arithmetic_conversion(lct, rct);
    if !common.is_half_float() {
        return None;
    }
    let is_bf16 = common == CType::BFloat16;
    let lb = half_bits_of(&cast_half_const(lv, Some(lct), &common)?)?;
    let rb = half_bits_of(&cast_half_const(rv, Some(rct), &common)?)?;
    let lf = if is_bf16 {
        hf::bf16_bits_to_f32(lb)
    } else {
        hf::f16_bits_to_f32(lb)
    };
    let rf = if is_bf16 {
        hf::bf16_bits_to_f32(rb)
    } else {
        hf::f16_bits_to_f32(rb)
    };
    let pack = |res: f32| -> IrConst {
        let b = if is_bf16 {
            hf::f32_to_bf16_bits(res)
        } else {
            hf::f32_to_f16_bits(res)
        };
        IrConst::I16(b as i16)
    };
    let truth = |b: bool| IrConst::I64(i64::from(b));
    Some(match op {
        BinOp::Add => pack(lf + rf),
        BinOp::Sub => pack(lf - rf),
        BinOp::Mul => pack(lf * rf),
        BinOp::Div => pack(lf / rf),
        BinOp::Lt => truth(lf < rf),
        BinOp::Le => truth(lf <= rf),
        BinOp::Gt => truth(lf > rf),
        BinOp::Ge => truth(lf >= rf),
        BinOp::Eq => truth(lf == rf),
        BinOp::Ne => truth(lf != rf),
        BinOp::LogicalAnd => truth((lb & 0x7fff != 0) && (rb & 0x7fff != 0)),
        BinOp::LogicalOr => truth((lb & 0x7fff != 0) || (rb & 0x7fff != 0)),
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

    // ---- strcmp/memcmp of string literals: the sign contract -------------
    //
    // C specifies only the SIGN of a compare result, and both reference
    // compilers fold a literal compare to -1/0/1 (measured on gcc 14.2 and
    // clang 23.1.3: memcmp("c","a",1) -> 1, memcmp("z","a",1) -> 1,
    // memcmp("\x80","\x7f",1) -> 1, strcmp("abc","abx") -> -1) while glibc's
    // runtime answer is a magnitude (2, 25, -21). These tests pin the folded
    // answer, the refusals, and the spelling/flag rule around it.

    fn cmp_lit(text: &str) -> Expr {
        Expr::StringLiteral(text.to_string(), Span::dummy())
    }

    fn cmp_int(value: i64) -> Expr {
        Expr::IntLiteral(value, Span::dummy())
    }

    fn cmp_eval_int(e: &Expr) -> Option<IrConst> {
        match e {
            Expr::IntLiteral(v, _) => Some(IrConst::I64(*v)),
            _ => None,
        }
    }

    fn cmp_call(name: &str, args: &[Expr]) -> Option<IrConst> {
        eval_builtin_call(name, args, &cmp_eval_int)
    }

    #[test]
    fn memcmp_of_literals_answers_with_the_sign_both_vendors_give() {
        let sign =
            |a: &str, b: &str, n: i64| cmp_call("memcmp", &[cmp_lit(a), cmp_lit(b), cmp_int(n)]);
        assert_eq!(sign("c", "a", 1), Some(IrConst::I32(1)));
        assert_eq!(sign("a", "c", 1), Some(IrConst::I32(-1)));
        assert_eq!(sign("z", "a", 1), Some(IrConst::I32(1)));
        // `unsigned char` comparison (C11 7.1.4p4 refers to the comparison
        // functions' byte-wise rule): 0x80 is GREATER than 0x7f, not negative.
        // A narrow literal stores one C byte per Rust char, so `\u{80}` here is
        // exactly the byte the C escape `"\x80"` produces (Rust string literals
        // are UTF-8 and reject `\x80` outright).
        assert_eq!(sign("\u{80}", "\u{7f}", 1), Some(IrConst::I32(1)));
        assert_eq!(sign("ab", "aa", 2), Some(IrConst::I32(1)));
        assert_eq!(sign("abc", "abx", 3), Some(IrConst::I32(-1)));
        assert_eq!(sign("abc", "abc", 3), Some(IrConst::I32(0)));
        // memcmp compares THROUGH an embedded NUL, unlike strcmp.
        assert_eq!(sign("a\0b", "a\0c", 3), Some(IrConst::I32(-1)));
        // n == 0 compares nothing: both vendors answer 0.
        assert_eq!(sign("abc", "xyz", 0), Some(IrConst::I32(0)));
    }

    #[test]
    fn strcmp_of_literals_stops_at_the_terminator() {
        let sign = |a: &str, b: &str| cmp_call("strcmp", &[cmp_lit(a), cmp_lit(b)]);
        assert_eq!(sign("c", "a"), Some(IrConst::I32(1)));
        assert_eq!(sign("a", "c"), Some(IrConst::I32(-1)));
        assert_eq!(sign("abc", "abx"), Some(IrConst::I32(-1)));
        assert_eq!(sign("abc", "abc"), Some(IrConst::I32(0)));
        // A prefix is ordered by the terminator, which is part of the object.
        assert_eq!(sign("ab", "abc"), Some(IrConst::I32(-1)));
        assert_eq!(sign("abc", "ab"), Some(IrConst::I32(1)));
        // The stored text ends at the first NUL, so what follows it is not
        // compared: strcmp, unlike memcmp, never looks past the terminator.
        assert_eq!(sign("a\0zzz", "a\0bbb"), Some(IrConst::I32(0)));
    }

    #[test]
    fn memcmp_declines_when_the_length_leaves_the_literals() {
        // One past either literal is out of the stored text; declining leaves
        // the call, which answers the same sign at runtime.
        assert_eq!(
            cmp_call("memcmp", &[cmp_lit("ab"), cmp_lit("abc"), cmp_int(4)]),
            None
        );
        assert_eq!(
            cmp_call("memcmp", &[cmp_lit("abc"), cmp_lit("ab"), cmp_int(3)]),
            None
        );
        // Exactly the stored length is inside it.
        assert_eq!(
            cmp_call("memcmp", &[cmp_lit("abc"), cmp_lit("abd"), cmp_int(3)]),
            Some(IrConst::I32(-1))
        );
    }

    #[test]
    fn memcmp_declines_a_negative_or_non_constant_length() {
        // A negative length is UB: no answer may be invented.
        assert_eq!(
            cmp_call("memcmp", &[cmp_lit("abc"), cmp_lit("abd"), cmp_int(-1)]),
            None
        );
        // A length the caller cannot prove constant declines too, and so does a
        // missing third argument.
        let opaque = Expr::Identifier("n".to_string(), Span::dummy());
        assert_eq!(
            cmp_call("memcmp", &[cmp_lit("abc"), cmp_lit("abd"), opaque]),
            None
        );
        assert_eq!(cmp_call("memcmp", &[cmp_lit("abc"), cmp_lit("abd")]), None);
    }

    #[test]
    fn strncmp_is_not_folded_because_the_vendors_disagree() {
        // gcc 14.2 folds strncmp("cd","ab",1) to glibc's magnitude 2 and clang
        // 23.1.3 folds it to the sign 1 (both measured at -O0 and -O2). With no
        // consensus to fold towards, the call stays and glibc answers.
        for name in ["strncmp", "__builtin_strncmp"] {
            assert_eq!(
                cmp_call(name, &[cmp_lit("cd"), cmp_lit("ab"), cmp_int(1)]),
                None,
                "{name} must not be folded"
            );
        }
    }

    #[test]
    fn the_prefixed_spelling_folds_when_the_bare_one_is_withdrawn() {
        let args = [cmp_lit("c"), cmp_lit("a"), cmp_int(1)];
        let both = [cmp_lit("c"), cmp_lit("a")];
        // Hosted mode: bare and prefixed agree.
        assert_eq!(
            eval_builtin_call_with_mode("memcmp", &args, &cmp_eval_int, true),
            Some(IrConst::I32(1))
        );
        assert_eq!(
            eval_builtin_call_with_mode("__builtin_memcmp", &args, &cmp_eval_int, true),
            Some(IrConst::I32(1))
        );
        // `-fno-builtin` / `-ffreestanding` withdraw the bare spelling only: the
        // compiler's own interface is not a library name to withdraw.
        assert_eq!(
            eval_builtin_call_with_mode("memcmp", &args, &cmp_eval_int, false),
            None
        );
        assert_eq!(
            eval_builtin_call_with_mode("__builtin_memcmp", &args, &cmp_eval_int, false),
            Some(IrConst::I32(1))
        );
        assert_eq!(
            eval_builtin_call_with_mode("strcmp", &both, &cmp_eval_int, false),
            None
        );
        assert_eq!(
            eval_builtin_call_with_mode("__builtin_strcmp", &both, &cmp_eval_int, false),
            Some(IrConst::I32(1))
        );
    }
}

/// Contract tests for the two C rules that the sema and lowering constant
/// folders must agree on. They live here, next to the single definition of
/// each rule, because the failure mode being guarded against is precisely
/// "one of the four call sites was not taught the rule".
#[cfg(test)]
mod promoted_fold_tests {
    use super::{
        bitnot_const_promoted, c_bool_const, c_bool_const_i128, c_bool_truth, c_bool_truth_i128,
        negate_const_promoted,
    };
    use crate::ir::constants::IrConst;

    #[test]
    fn bool_conversion_is_by_value_not_by_low_byte() {
        // Every value here has a zero low byte or a zero low word; a
        // truncate-then-normalise fold answers 0 for all of them.
        for nonzero in [
            256u64,
            65536,
            67_106_816,
            1 << 24,
            1 << 32,
            u64::MAX,
            0x8000_0000,
        ] {
            assert_eq!(c_bool_const(nonzero), IrConst::I32(1), "(_Bool){nonzero}");
            assert!(c_bool_truth(nonzero));
        }
        assert_eq!(c_bool_const(0), IrConst::I32(0));
        assert!(!c_bool_truth(0));
    }

    #[test]
    fn bool_conversion_sees_the_whole_int128() {
        // (unsigned __int128)1 << 100 has zero low 64 bits but is nonzero.
        let high_only = 1i128 << 100;
        assert_eq!(c_bool_const_i128(high_only), IrConst::I32(1));
        assert!(c_bool_truth_i128(high_only));
        assert_eq!(c_bool_const_i128(0), IrConst::I32(0));
        // A value whose low 64 bits are all ones is also just "nonzero".
        assert_eq!(c_bool_const_i128(i128::MIN), IrConst::I32(1));
    }

    #[test]
    fn unary_minus_of_unsigned_int_is_mod_2_32() {
        // The YARPGen seed-101 value: -(3844603080U) == 450364216 (mod 2^32).
        assert_eq!(
            negate_const_promoted(IrConst::I64(3_844_603_080), true, Some(4)),
            Some(IrConst::I64(450_364_216))
        );
        // -(42U) == 4294967254 (mod 2^32), not 2^64-42.
        assert_eq!(
            negate_const_promoted(IrConst::I64(42), true, Some(4)),
            Some(IrConst::I64(4_294_967_254))
        );
        // -(0U) stays 0.
        assert_eq!(
            negate_const_promoted(IrConst::I64(0), true, Some(4)),
            Some(IrConst::I64(0))
        );
        // -(2^31 U) == 2^31 (mod 2^32), the wraparound boundary.
        assert_eq!(
            negate_const_promoted(IrConst::I64(1 << 31), true, Some(4)),
            Some(IrConst::I64(1 << 31))
        );
    }

    #[test]
    fn unary_minus_of_a_signed_int_keeps_its_sign_in_the_carrier() {
        // The carrier is 64-bit because the *surrounding* expression is
        // 64-bit, not because the operand is unsigned. `(size_t)-(enum{3})`
        // must fold to -3 so the cast sign-extends it to 2^64-3; masking
        // the carrier to 32 bits here produced 4294967293 (verified
        // against GCC 16.2 and Clang 23.1.3).
        assert_eq!(
            negate_const_promoted(IrConst::I64(3), false, Some(4)),
            Some(IrConst::I64(-3))
        );
        assert_eq!(
            negate_const_promoted(IrConst::I64(-3), false, Some(4)),
            Some(IrConst::I64(3))
        );
        // `~0` as a signed int is -1, the all-ones 64-bit pattern; masking
        // would give 4294967295, which a later 64-bit use reads as a
        // positive value.
        assert_eq!(
            bitnot_const_promoted(IrConst::I64(0), false, Some(4)),
            Some(IrConst::I64(-1))
        );
        assert_eq!(
            bitnot_const_promoted(IrConst::I64(41), false, Some(4)),
            Some(IrConst::I64(-42))
        );
    }

    #[test]
    fn unary_minus_of_unsigned_long_stays_mod_2_64() {
        // An 8-byte operand must keep the carrier rule; masking here would
        // truncate a real 64-bit value. -(42UL) is 2^64-42, which is the
        // i64 bit pattern -42.
        assert_eq!(
            negate_const_promoted(IrConst::I64(42), true, Some(8)),
            Some(IrConst::I64(-42))
        );
        assert_eq!(
            negate_const_promoted(IrConst::I64(42), true, None),
            Some(IrConst::I64(-42)),
            "an unknown operand size must not license a 32-bit reduction"
        );
    }

    #[test]
    fn unary_minus_of_signed_sub_int_promotes_to_int() {
        // C11 6.3.1.1: (signed char)-128 promotes to int -128, so -(-128)
        // is 128 as an int, not a byte wrapping to -128.
        assert_eq!(
            negate_const_promoted(IrConst::I8(-128), false, Some(1)),
            Some(IrConst::I32(128))
        );
        // (unsigned char)200 promotes to int 200.
        assert_eq!(
            negate_const_promoted(IrConst::I8(-56), true, Some(1)),
            Some(IrConst::I32(-200))
        );
        assert_eq!(
            negate_const_promoted(IrConst::I16(-1), true, Some(2)),
            Some(IrConst::I32(-65535))
        );
    }

    #[test]
    fn unary_minus_leaves_floats_alone() {
        assert_eq!(
            negate_const_promoted(IrConst::F64(1.5), false, Some(8)),
            Some(IrConst::F64(-1.5))
        );
        // -0.0 must stay -0.0 (sign-bit flip, not arithmetic negation).
        assert_eq!(
            negate_const_promoted(IrConst::F64(0.0), false, Some(8)),
            Some(IrConst::F64(-0.0))
        );
    }

    #[test]
    fn bitwise_not_of_unsigned_int_is_mod_2_32() {
        assert_eq!(
            bitnot_const_promoted(IrConst::I64(0xFFFF_FFFF), true, Some(4)),
            Some(IrConst::I64(0))
        );
        assert_eq!(
            bitnot_const_promoted(IrConst::I64(0), true, Some(4)),
            Some(IrConst::I64(0xFFFF_FFFF))
        );
        // 8-byte operand: complement the full carrier.
        assert_eq!(
            bitnot_const_promoted(IrConst::I64(0), true, Some(8)),
            Some(IrConst::I64(-1))
        );
        // Sub-int promotes to int: ~(unsigned char)0 is -1 as an int.
        assert_eq!(
            bitnot_const_promoted(IrConst::I8(0), true, Some(1)),
            Some(IrConst::I32(-1))
        );
    }

    #[test]
    fn int128_carrier_wraps_in_its_own_width() {
        // -(unsigned __int128)1 is all-ones, i.e. i128 -1: wrapping_neg on
        // the carrier already reduces mod 2^128, so no extra mask applies.
        assert_eq!(
            negate_const_promoted(IrConst::I128(1), true, Some(16)),
            Some(IrConst::I128(-1))
        );
        assert_eq!(
            bitnot_const_promoted(IrConst::I128(0), true, Some(16)),
            Some(IrConst::I128(-1))
        );
    }
}
