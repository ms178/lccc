//! Property-based tests for the shared constant-evaluation helpers.
//!
//! Covers (all in `common::const_eval` unless noted):
//!   - `eval_literal`
//!   - `eval_builtin_call`
//!   - `eval_binop_with_types`
//!   - `promote_sub_int`
//!   - `irconst_to_bits`
//!   - `truncate_and_extend_bits`  (in `common::const_arith`)
//!
//! This file is pulled in as a `#[cfg(test)]` submodule of `const_eval` via
//! `#[path = "const_eval_pbt.rs"]` (see the bottom of `const_eval.rs`), so it
//! reaches private items through `super::` and `crate::`. Properties target the
//! *real* production functions and use implementation-independent oracles
//! (algebraic identities, metamorphic laws, negative contracts) rather than
//! re-stating each function's own arithmetic.

use lccc::pbt_internals::IrConst;
use lccc::pbt_internals::Span;
use lccc::pbt_internals::truncate_and_extend_bits;
use lccc::pbt_internals::{BinOp, Expr};
use lccc::pbt_internals::{
    eval_binop_with_types, eval_builtin_call, eval_literal, irconst_to_bits, promote_sub_int,
};
use proptest::prelude::*;

fn span() -> Span {
    Span::dummy()
}

fn int_lit(x: i64) -> Expr {
    Expr::IntLiteral(x, span())
}

/// Structural equality for `IrConst` (no `PartialEq` derive) via its `Debug` repr.
fn same<T: std::fmt::Debug>(a: T, b: T) -> bool {
    format!("{a:?}") == format!("{b:?}")
}

/// Evaluate a unary integer builtin against a *fixed* integer operand `x`.
/// The builtin's own `eval_fn` closure always yields `Some(I64(x))`, which
/// decouples the builtin arithmetic under test from `eval_literal`.
fn eval_int_builtin(name: &str, x: i64) -> Option<IrConst> {
    let f = |_: &Expr| Some(IrConst::I64(x));
    eval_builtin_call(name, &[int_lit(0)], &f)
}

// ===========================================================================
// eval_literal
// ===========================================================================

proptest! {
    // Oracle: algebraic — value round-trip + width selection.
    // forall v: i64. to_i64(eval_literal(IntLiteral(v))) == v, and the variant
    // is I32 iff v fits in i32, else I64.
    #[test]
    fn int_literal_round_trips_and_picks_width(v in any::<i64>()) {
        let got = eval_literal(&int_lit(v)).expect("IntLiteral always evaluates");
        prop_assert_eq!(got.to_i64(), Some(v));
        let in_i32 = (i32::MIN as i64..=i32::MAX as i64).contains(&v);
        match (in_i32, got) {
            (true, IrConst::I32(_)) | (false, IrConst::I64(_)) => {}
            other => prop_assert!(false, "wrong variant for v={v}: {other:?}"),
        }
    }

    // Oracle: algebraic — signed long / long-long literals preserve the value as I64.
    #[test]
    fn signed_long_literals_preserve_value(v in any::<i64>()) {
        for e in [Expr::LongLiteral(v, span()), Expr::LongLongLiteral(v, span())] {
            let got = eval_literal(&e).unwrap();
            prop_assert!(matches!(got, IrConst::I64(_)));
            prop_assert_eq!(got.to_i64(), Some(v));
        }
    }

    // Oracle: algebraic — unsigned literals preserve the exact bit pattern.
    #[test]
    fn unsigned_int_literals_preserve_bit_pattern(v in any::<u64>()) {
        for e in [
            Expr::UIntLiteral(v, span()),
            Expr::ULongLiteral(v, span()),
            Expr::ULongLongLiteral(v, span()),
        ] {
            let got = eval_literal(&e).unwrap();
            prop_assert!(matches!(got, IrConst::I64(_)));
            // bits preserved through the i64 reinterpretation
            prop_assert_eq!(got.to_i64().unwrap() as u64, v);
        }
    }

    // Oracle: reference — a char literal sign-extends its byte to int (C semantics).
    #[test]
    fn char_literal_sign_extends_the_byte(code in 0u32..256u32) {
        let ch = char::from_u32(code).unwrap();
        let got = eval_literal(&Expr::CharLiteral(ch, span())).unwrap();
        let b = code as u8;
        let expected = (b as i8) as i32; // signed char -> int
        match got {
            IrConst::I32(v) => prop_assert_eq!(v, expected),
            other => prop_assert!(false, "char literal must be I32, got {other:?}"),
        }
        // low byte preserved regardless of sign extension
        prop_assert_eq!(got.to_i64().unwrap() as u64 & 0xFF, b as u64);
    }

    // Oracle: reference — f64/f32 literals preserve the exact bit pattern (incl. NaN/Inf).
    #[test]
    fn float_literal_preserves_bit_pattern(bits in any::<u64>()) {
        let v = f64::from_bits(bits);
        let got = eval_literal(&Expr::FloatLiteral(v, span())).unwrap();
        match got {
            IrConst::F64(g) => prop_assert_eq!(g.to_bits(), bits),
            other => prop_assert!(false, "float literal must be F64, got {other:?}"),
        }
    }

    #[test]
    fn float32_literal_preserves_value(v in any::<f64>()) {
        let got = eval_literal(&Expr::FloatLiteralF32(v, span())).unwrap();
        match got {
            IrConst::F32(g) => prop_assert_eq!(g.to_bits(), (v as f32).to_bits()),
            other => prop_assert!(false, "f32 literal must be F32, got {other:?}"),
        }
    }
}

#[test]
fn non_literal_expressions_evaluate_to_none() {
    // An identifier is not a literal -> None (this is what __builtin_constant_p relies on).
    let id = Expr::Identifier("x".to_string(), span());
    assert!(eval_literal(&id).is_none());
}

// ===========================================================================
// eval_builtin_call
// ===========================================================================

proptest! {
    // Oracle: algebraic — bswap is an involution on its target width:
    //   bswap_N(bswap_N(x)) == x  (mod N bits), for N in {16, 32, 64}.
    #[test]
    fn bswap_is_an_involution(width in 0u8..3, x in any::<i64>()) {
        let (name, mask): (&str, u64) = match width {
            0 => ("__builtin_bswap16", 0xFFFF),
            1 => ("__builtin_bswap32", 0xFFFF_FFFF),
            _ => ("__builtin_bswap64", u64::MAX),
        };
        let r1 = eval_int_builtin(name, x).unwrap();
        let r2 = eval_int_builtin(name, r1.to_i64().unwrap()).unwrap();
        prop_assert_eq!(r2.to_i64().unwrap() as u64 & mask, x as u64 & mask);
    }

    // Oracle: metamorphic — popcount is invariant under byte reversal:
    //   popcount(bswap_N(x)) == popcount_N(x).
    #[test]
    fn popcount_is_invariant_under_bswap(width in 0u8..2, x in any::<i64>()) {
        let (bswap, popc): (&str, &str) = match width {
            0 => ("__builtin_bswap32", "__builtin_popcount"),
            _ => ("__builtin_bswap64", "__builtin_popcountll"),
        };
        let plain = eval_int_builtin(popc, x).unwrap().to_i64().unwrap();
        let swapped = eval_int_builtin(bswap, x).unwrap().to_i64().unwrap();
        let after = eval_int_builtin(popc, swapped).unwrap().to_i64().unwrap();
        prop_assert_eq!(plain, after);
    }

    // Oracle: algebraic — parity(x) == popcount(x) (mod 2), and parity in {0,1}.
    #[test]
    fn parity_equals_popcount_mod_two(width in 0u8..2, x in any::<i64>()) {
        let (par, popc): (&str, &str) = match width {
            0 => ("__builtin_parity", "__builtin_popcount"),
            _ => ("__builtin_parityll", "__builtin_popcountll"),
        };
        let p = eval_int_builtin(par, x).unwrap().to_i64().unwrap();
        let c = eval_int_builtin(popc, x).unwrap().to_i64().unwrap();
        prop_assert_eq!(p, c & 1);
        prop_assert!(p == 0 || p == 1);
    }

    // Oracle: algebraic — clz/ctz/popcount correctly identify the set bits.
    // For u != 0: clz gives the highest set bit position (hsb = width-1-clz),
    // ctz gives the lowest set bit position, and popcount is in [1, width].
    // For u == 0 the SUT convention is clz=w, ctz=w, popcount=0.
    #[test]
    fn clz_ctz_popcount_pin_the_set_bits(width in 0u8..2, x in any::<i64>()) {
        let (clz, ctz, popc, w, mask): (&str, &str, &str, u32, u64) = match width {
            0 => ("__builtin_clz", "__builtin_ctz", "__builtin_popcount", 32, 0xFFFF_FFFF),
            _ => ("__builtin_clzll", "__builtin_ctzll", "__builtin_popcountll", 64, u64::MAX),
        };
        let u = x as u64 & mask;
        let cl = eval_int_builtin(clz, x).unwrap().to_i64().unwrap();
        let ct = eval_int_builtin(ctz, x).unwrap().to_i64().unwrap();
        let po = eval_int_builtin(popc, x).unwrap().to_i64().unwrap();
        if u == 0 {
            prop_assert_eq!(cl, w as i64);
            prop_assert_eq!(ct, w as i64);
            prop_assert_eq!(po, 0);
        } else {
            let hsb = (w - 1) - cl as u32; // highest set bit position
            prop_assert_eq!(u >> hsb, 1u64); // bit hsb set, nothing above it
            prop_assert_eq!((u >> ct) & 1, 1u64);   // bit ct is set
            if ct > 0 {
                prop_assert_eq!(u & ((1u64 << ct) - 1), 0u64); // nothing below ct
            }
            prop_assert!((1..=w as i64).contains(&po));
            prop_assert!(hsb >= ct as u32); // MSB at/above LSB
        }
    }

    // Oracle: algebraic — ffs(x) == ctz(x) + 1 for x != 0, and ffs(0) == 0.
    #[test]
    fn ffs_is_one_plus_ctz_for_nonzero(width in 0u8..2, x in any::<i64>()) {
        let (ffs, ctz, mask, w): (&str, &str, u64, i64) = match width {
            0 => ("__builtin_ffs", "__builtin_ctz", 0xFFFF_FFFF, 32),
            _ => ("__builtin_ffsll", "__builtin_ctzll", u64::MAX, 64),
        };
        let f = eval_int_builtin(ffs, x).unwrap().to_i64().unwrap();
        let c = eval_int_builtin(ctz, x).unwrap().to_i64().unwrap();
        if x as u64 & mask == 0 {
            prop_assert_eq!(f, 0);
        } else {
            prop_assert_eq!(f, c + 1);
            prop_assert!((1..=w).contains(&f));
        }
    }

    // Oracle: algebraic — clrsb(x) == clrsb(~x) (sign-bit count is preserved
    // when every bit flips), and clrsb stays in [0, width-1].
    #[test]
    fn clrsb_is_invariant_under_bitwise_not(width in 0u8..2, x in any::<i64>()) {
        let (name, max): (&str, i64) = match width {
            0 => ("__builtin_clrsb", 31),
            _ => ("__builtin_clrsbll", 63),
        };
        let a = eval_int_builtin(name, x).unwrap().to_i64().unwrap();
        let b = eval_int_builtin(name, !x).unwrap().to_i64().unwrap();
        prop_assert_eq!(a, b);
        prop_assert!((0..=max).contains(&a));
    }

    // Oracle: negative/error contract — integer bit builtins reject float
    // operands (to_i64() of a float is None, so the builtin must return None).
    #[test]
    fn integer_builtins_return_none_for_float_operands(x in any::<i64>()) {
        let f = move |_: &Expr| -> Option<IrConst> { Some(IrConst::F64(x as f64)) };
        for name in [
            "__builtin_clz",
            "__builtin_ctz",
            "__builtin_popcount",
            "__builtin_bswap32",
            "__builtin_ffs",
            "__builtin_clrsb",
        ] {
            let r = eval_builtin_call(name, &[int_lit(0)], &f);
            prop_assert!(r.is_none(), "{name} on a float operand must be None, got {r:?}");
        }
    }

    // Oracle: reference — __builtin_choose_expr picks arg1 when cond != 0, else arg2.
    #[test]
    fn choose_expr_branches_on_cond_nonzero(cond in any::<i64>(), a in any::<i64>(), b in any::<i64>()) {
        let f = |e: &Expr| -> Option<IrConst> { eval_literal(e) };
        let args = [int_lit(cond), int_lit(a), int_lit(b)];
        let got = eval_builtin_call("__builtin_choose_expr", &args, &f).unwrap();
        let expected = if cond != 0 { eval_literal(&int_lit(a)) } else { eval_literal(&int_lit(b)) };
        prop_assert!(same(got, expected.unwrap()));
    }

    // Oracle: reference — __builtin_expect returns the evaluation of its first arg.
    #[test]
    fn expect_returns_first_arg(v in any::<i64>()) {
        let f = |e: &Expr| -> Option<IrConst> { eval_literal(e) };
        let args = [int_lit(v), int_lit(1)];
        let got = eval_builtin_call("__builtin_expect", &args, &f).unwrap();
        prop_assert!(same(got, eval_literal(&int_lit(v)).unwrap()));
    }
}

#[test]
fn constant_p_returns_one_for_constant_and_defers_zero() {
    // Contract (lccc main, documented at the eval_builtin_call site): a
    // provable constant yields Some(1); a not-yet-provable argument yields
    // None rather than Some(0), because a local/parameter may become a
    // constant after mem2reg/inlining/propagation and an early zero would
    // be cached as a folded constant. The invariant that must hold either
    // way: a non-constant argument can never produce Some(1).
    let f = |e: &Expr| -> Option<IrConst> { eval_literal(e) };
    let one = eval_builtin_call("__builtin_constant_p", &[int_lit(42)], &f).unwrap();
    assert!(matches!(one, IrConst::I32(1)));
    let id = Expr::Identifier("x".to_string(), span());
    let not_const = eval_builtin_call("__builtin_constant_p", &[id], &f);
    match not_const {
        None => {}
        Some(IrConst::I32(0)) => {}
        other => {
            panic!("__builtin_constant_p(non-constant) must be None or Some(0), got {other:?}")
        }
    }
}

#[test]
fn float_class_builtins_produce_nan_and_infinity() {
    // These arms ignore eval_fn and the argument entirely.
    assert!(matches!(eval_int_builtin("__builtin_nan", 0), Some(IrConst::F64(v)) if v.is_nan()));
    assert!(
        matches!(eval_int_builtin("__builtin_inf", 0), Some(IrConst::F64(v)) if v.is_infinite() && v > 0.0)
    );
    assert!(matches!(eval_int_builtin("__builtin_nanf", 0), Some(IrConst::F32(v)) if v.is_nan()));
    assert!(
        matches!(eval_int_builtin("__builtin_inff", 0), Some(IrConst::F32(v)) if v.is_infinite() && v > 0.0)
    );
    assert!(
        matches!(eval_int_builtin("__builtin_huge_val", 0), Some(IrConst::F64(v)) if v.is_infinite() && v > 0.0)
    );
    assert!(
        matches!(eval_int_builtin("__builtin_huge_valf", 0), Some(IrConst::F32(v)) if v.is_infinite() && v > 0.0)
    );
}

// ===========================================================================
// promote_sub_int
// ===========================================================================

proptest! {
    // Oracle: reference (C11 6.3.1.1) — sub-int (I8/I16) always promotes to I32.
    #[test]
    fn sub_int_promotion_yields_i32(
        variant in 0u8..2, v8 in any::<i8>(), v16 in any::<i16>(), unsigned in any::<bool>(),
    ) {
        let val = if variant == 0 { IrConst::I8(v8) } else { IrConst::I16(v16) };
        let got = promote_sub_int(val, unsigned);
        prop_assert!(matches!(got, IrConst::I32(_)), "expected I32, got {got:?}");
    }

    // Oracle: algebraic — the low 8/16 bits are preserved regardless of signedness.
    #[test]
    fn sub_int_low_bits_preserved_regardless_of_signedness(
        variant in 0u8..2, v8 in any::<i8>(), v16 in any::<i16>(), unsigned in any::<bool>(),
    ) {
        let (val, keep): (IrConst, u64) = if variant == 0 {
            (IrConst::I8(v8), 0xFF)
        } else {
            (IrConst::I16(v16), 0xFFFF)
        };
        let got = promote_sub_int(val, unsigned).to_i64().unwrap() as u64;
        let original = val.to_i64().unwrap() as u64;
        prop_assert_eq!(got & keep, original & keep);
    }

    // Oracle: reference — unsigned promotion zero-extends into [0, 2^keep).
    #[test]
    fn unsigned_promotion_zero_extends(variant in 0u8..2, v8 in any::<i8>(), v16 in any::<i16>()) {
        let (val, limit, keep): (IrConst, i64, u64) = if variant == 0 {
            (IrConst::I8(v8), 255, 0xFF)
        } else {
            (IrConst::I16(v16), 65535, 0xFFFF)
        };
        let got = promote_sub_int(val, true).to_i64().unwrap();
        prop_assert!((0..=limit).contains(&got));
        prop_assert_eq!(got as u64 & keep, val.to_i64().unwrap() as u64 & keep);
    }

    // Oracle: reference — signed promotion sign-extends (value unchanged as i64).
    #[test]
    fn signed_promotion_sign_extends(variant in 0u8..2, v8 in any::<i8>(), v16 in any::<i16>()) {
        let val = if variant == 0 { IrConst::I8(v8) } else { IrConst::I16(v16) };
        let got = promote_sub_int(val, false).to_i64().unwrap();
        prop_assert_eq!(got, val.to_i64().unwrap());
    }

    // Oracle: algebraic — promotion is idempotent.
    #[test]
    fn promote_sub_int_is_idempotent(
        variant in 0u8..4, v8 in any::<i8>(), v16 in any::<i16>(),
        v32 in any::<i32>(), v64 in any::<i64>(), unsigned in any::<bool>(),
    ) {
        let val = match variant {
            0 => IrConst::I8(v8),
            1 => IrConst::I16(v16),
            2 => IrConst::I32(v32),
            _ => IrConst::I64(v64),
        };
        let once = promote_sub_int(val, unsigned);
        let twice = promote_sub_int(once, unsigned);
        prop_assert!(same(once, twice));
    }

    // Oracle: algebraic — non-sub-int constants (I32/I64/I128/floats/Zero) pass through unchanged.
    #[test]
    fn non_sub_int_constants_pass_through_unchanged(
        variant in 0u8..5, v32 in any::<i32>(), v64 in any::<i64>(), v128 in any::<i128>(),
        f32bits in any::<u32>(), f64bits in any::<u64>(), unsigned in any::<bool>(),
    ) {
        let val: IrConst = match variant {
            0 => IrConst::I32(v32),
            1 => IrConst::I64(v64),
            2 => IrConst::I128(v128),
            3 => IrConst::F32(f32::from_bits(f32bits)),
            _ => IrConst::F64(f64::from_bits(f64bits)),
        };
        let got = promote_sub_int(val, unsigned);
        prop_assert!(same(got, val));
        prop_assert!(same(promote_sub_int(IrConst::Zero, unsigned), IrConst::Zero));
    }
}

// ===========================================================================
// irconst_to_bits
// ===========================================================================

proptest! {
    // Oracle: algebraic — every integer variant (and Zero) maps bits to
    // to_i64() as u64, and the signedness flag is always true.
    #[test]
    fn integer_variants_map_bits_to_to_i64(
        variant in 0u8..6, v8 in any::<i8>(), v16 in any::<i16>(), v32 in any::<i32>(),
        v64 in any::<i64>(), v128 in any::<i128>(),
    ) {
        let val: IrConst = match variant {
            0 => IrConst::I8(v8),
            1 => IrConst::I16(v16),
            2 => IrConst::I32(v32),
            3 => IrConst::I64(v64),
            4 => IrConst::I128(v128),
            _ => IrConst::Zero,
        };
        let (bits, is_signed) = irconst_to_bits(&val);
        prop_assert!(is_signed);
        prop_assert_eq!(bits, val.to_i64().unwrap_or(0) as u64);
    }

    // Contract assertion (NOT a bug — intentional value-based conversion):
    // irconst_to_bits maps F64 to `v as i64 as u64` (value truncated toward
    // zero), NOT the IEEE bit pattern, despite the doc string's loose phrase
    // "raw bit representation". This is correct and by design:
    //   - Spec evidence: C11 6.3.1.4 — a floating-to-integer conversion truncates
    //     toward zero (a *value* conversion), which is exactly `v as i64`.
    //   - Design evidence: ir/lowering/const_eval.rs `eval_const_cast` intercepts
    //     float sources BEFORE the bits path with the comment "Handle float
    //     source types: use value-based conversion, not bit manipulation"; float
    //     operands only reach irconst_to_bits on that value-conversion path.
    //   - The doc string itself qualifies "raw u64 bits ... with fallback to
    //     value conversion".
    //   - IEEE bits would be WRONG: `(int)((unsigned long)(1.5))` must be 1, but
    //     IEEE bits (0x3FF8_0000_0000_0000) truncate to low-32 = 0.
    #[test]
    fn f64_irconst_to_bits_is_value_conversion_per_c11(v in any::<f64>()) {
        let (bits, is_signed) = irconst_to_bits(&IrConst::F64(v));
        prop_assert!(is_signed);
        // Value-based conversion toward zero (C11 6.3.1.4), not IEEE bit pattern.
        prop_assert_eq!(bits, v as i64 as u64);
        // For any nonzero value the two encodings differ, proving this is value
        // conversion, not bit reinterpretation. (They coincide at 0.0.)
        if v != 0.0 {
            prop_assert_ne!(bits, v.to_bits(), "must NOT be the IEEE bit pattern");
        }
    }

    // Same value-conversion contract (C11 6.3.1.4) for F32.
    #[test]
    fn f32_irconst_to_bits_is_value_conversion_per_c11(v in any::<f32>()) {
        let (bits, is_signed) = irconst_to_bits(&IrConst::F32(v));
        prop_assert!(is_signed);
        prop_assert_eq!(bits, v as i64 as u64);
        if v != 0.0 {
            prop_assert_ne!(bits, v.to_bits() as u64, "must NOT be the IEEE bit pattern");
        }
    }
}

// ===========================================================================
// eval_binop_with_types
// ===========================================================================

proptest! {
    // Oracle: negative/error contract — integer Div/Mod by zero must be None
    // for every width/signedness combination (C: division by zero is UB; the
    // compiler must signal it rather than produce a value).
    #[test]
    fn div_mod_by_zero_return_none(
        op_is_div in any::<bool>(), lhs in any::<i64>(),
        lhs_size in prop_oneof![Just(4usize), Just(8usize)],
        rhs_size in prop_oneof![Just(4usize), Just(8usize)],
        lhs_unsigned in any::<bool>(), rhs_unsigned in any::<bool>(),
    ) {
        let op = if op_is_div { BinOp::Div } else { BinOp::Mod };
        let r = eval_binop_with_types(
            &op, &IrConst::I64(lhs), &IrConst::I64(0),
            lhs_size, lhs_unsigned, rhs_size, rhs_unsigned,
        );
        prop_assert!(r.is_none(), "div/mod by zero must be None, got {r:?}");
    }

    // Oracle: reference (C11 6.5.7) — a shift's result type depends only on the
    // LHS; widening/changing the signedness of the RHS must not change the result.
    #[test]
    fn shift_result_is_independent_of_rhs_type(
        x in any::<i64>(),
        lhs_size in prop_oneof![Just(4usize), Just(8usize)],
        lhs_unsigned in any::<bool>(),
    ) {
        let narrow = eval_binop_with_types(
            &BinOp::Shl, &IrConst::I64(x), &IrConst::I64(1),
            lhs_size, lhs_unsigned, 4, false,
        );
        let wide_unsigned_rhs = eval_binop_with_types(
            &BinOp::Shl, &IrConst::I64(x), &IrConst::I64(1),
            lhs_size, lhs_unsigned, 8, true,
        );
        prop_assert!(same(narrow, wide_unsigned_rhs));
    }
}

// ===========================================================================
// truncate_and_extend_bits (const_arith.rs)
// ===========================================================================

proptest! {
    // Oracle: reference — width == 64 is the no-op boundary (the `>= 64` branch).
    #[test]
    fn width_64_is_a_noop(bits in any::<u64>(), signed in any::<bool>()) {
        let (r, s) = truncate_and_extend_bits(bits, 64, signed);
        prop_assert_eq!(r, bits);
        prop_assert_eq!(s, signed);
    }

    // Oracle: reference — for standard widths (8/16/32), unsigned results match
    // the masked low bits, and signed results match two's-complement sign extension.
    #[test]
    fn truncate_and_extend_matches_known_widths(bits in any::<u64>(), log_width in 3u32..6) {
        let width = 1usize << log_width; // 8, 16, 32
        let mask = (1u64 << width) - 1;

        let (unsigned_r, _) = truncate_and_extend_bits(bits, width, false);
        prop_assert_eq!(unsigned_r, bits & mask);

        let (signed_r, _) = truncate_and_extend_bits(bits, width, true);
        let low = bits & mask;
        let sign = 1u64 << (width - 1);
        let expected = if low & sign != 0 { low | !mask } else { low };
        prop_assert_eq!(signed_r, expected);
    }
}
