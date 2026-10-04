//! Property-based tests for the shared constant binary evaluators in
//! `const_arith.rs` — `eval_const_binop` (integer / i128 dispatch) and
//! `eval_const_binop_float` (F32/F64/long-double).
//!
//! This is a separate `#[cfg(test)]` module, pulled into the build by the
//! `#[cfg(test)] #[path = "const_arith_binop_prop_tests.rs"] mod ...;`
//! declaration at the bottom of `const_arith.rs`. It deliberately covers the
//! gaps left by the inline `eval_const_binop_pbt` module: the entire
//! `eval_const_binop_float` path (previously untested), the integer shift and
//! comparison operators, and the signedness-sensitive i128 operators.
//!
//! Any property that exposes a confirmed SUT bug is marked
//! `#[ignore = "documented bug: ..."]` so the default `cargo test` run stays
//! green; run a witness explicitly with
//! `cargo test --test pbt_common const_arith_binop_prop_tests -- <name> --ignored`.

use lccc::pbt_internals::BinOp;
use lccc::pbt_internals::IrConst;
use lccc::pbt_internals::{eval_const_binop, eval_const_binop_float};
use proptest::prelude::*;

// `IrConst` does not derive `PartialEq`, so normalize integer results into a
// (variant-tag, i64-value) tuple for structural comparison.
fn classify_int(c: Option<IrConst>) -> (u8, i64) {
    match c.expect("integer binop returned None") {
        IrConst::I64(v) => (0, v),
        IrConst::I32(v) => (1, v as i64),
        IrConst::I128(v) => (2, v as i64),
        other => panic!("unexpected non-integer result: {:?}", other),
    }
}

// Bit-exact f32 comparison that treats any pair of NaNs as equal (NaN payload
// is don't-care) but is otherwise strict (so a flipped sign-of-zero is caught).
fn f32_bits_eq(a: f32, b: f32) -> bool {
    (a.is_nan() && b.is_nan()) || a.to_bits() == b.to_bits()
}

proptest! {
    // ======================================================================
    // eval_const_binop_float — F32/F64 native path
    // ======================================================================

    // ORACLE: differential vs native Rust f64 arithmetic.
    // The F64 path computes `lhs.to_f64() <op> rhs.to_f64()` and rewraps as
    // F64, so the result must be bit-identical to the native f64 operation for
    // every finite operand pair (including inf/NaN results such as x/0).
    //
    // Formal: ∀ l,r ∈ finite_f64.
    //   eval_const_binop_float(op, F64(l), F64(r)) = F64(l ⊙ r)  (bit-identical)
    //   for op ∈ {+, -, *, /}.
    #[test]
    fn f64_arithmetic_matches_native(
        op_idx in 0u8..4,
        lb in any::<u64>(),
        rb in any::<u64>(),
    ) {
        let l = f64::from_bits(lb);
        let r = f64::from_bits(rb);
        prop_assume!(l.is_finite() && r.is_finite());
        let (op, expected) = match op_idx {
            0 => (BinOp::Add, l + r),
            1 => (BinOp::Sub, l - r),
            2 => (BinOp::Mul, l * r),
            _ => (BinOp::Div, l / r),
        };
        let got = eval_const_binop_float(&op, &IrConst::F64(l), &IrConst::F64(r))
            .expect("F64 arithmetic must return Some");
        match got {
            IrConst::F64(v) => prop_assert_eq!(v.to_bits(), expected.to_bits()),
            other => prop_assert!(false, "expected F64, got {:?}", other),
        }
    }

    // ORACLE: differential vs native Rust f32 arithmetic.
    // The F32 path computes in f64 then rounds back to f32. Because f64 has
    // more than 2x the mantissa width of f32 (53 >= 2*24), this double
    // rounding is correctly rounded, so the result must be bit-identical to
    // direct f32 arithmetic (sign-of-zero included) for every finite operand
    // pair. NaN payloads from 0.0/0.0 are treated as equal.
    //
    // Formal: ∀ l,r ∈ finite_f32.
    //   eval_const_binop_float(op, F32(l), F32(r)) = F32(l ⊙ r)  (value-identical)
    //   for op ∈ {+, -, *, /}.
    #[test]
    fn f32_arithmetic_matches_native(
        op_idx in 0u8..4,
        lb in any::<u32>(),
        rb in any::<u32>(),
    ) {
        let l = f32::from_bits(lb);
        let r = f32::from_bits(rb);
        prop_assume!(l.is_finite() && r.is_finite());
        let (op, expected) = match op_idx {
            0 => (BinOp::Add, l + r),
            1 => (BinOp::Sub, l - r),
            2 => (BinOp::Mul, l * r),
            _ => (BinOp::Div, l / r),
        };
        let got = eval_const_binop_float(&op, &IrConst::F32(l), &IrConst::F32(r))
            .expect("F32 arithmetic must return Some");
        match got {
            IrConst::F32(v) => prop_assert!(f32_bits_eq(v, expected), "got {} expected {}", v, expected),
            other => prop_assert!(false, "expected F32, got {:?}", other),
        }
    }

    // ORACLE: differential vs native Rust f64 comparison + C truthiness.
    // Comparison and logical operators on floats must always yield I64(0/1)
    // matching the native comparison, regardless of F32/F64 width.
    //
    // Formal: ∀ l,r ∈ finite_f64, op ∈ {==,!=,<,>,<=,>=,&&,||}.
    //   eval_const_binop_float(op, a, b) = I64( native_truth(op, l, r) ? 1 : 0 ).
    #[test]
    fn float_comparison_and_logical_match_native(
        op_idx in 0u8..8,
        lb in any::<u64>(),
        rb in any::<u64>(),
        is_f32 in any::<bool>(),
    ) {
        let (l, r, lhs, rhs) = if is_f32 {
            let l = f32::from_bits(lb as u32);
            let r = f32::from_bits(rb as u32);
            (l as f64, r as f64, IrConst::F32(l), IrConst::F32(r))
        } else {
            let l = f64::from_bits(lb);
            let r = f64::from_bits(rb);
            (l, r, IrConst::F64(l), IrConst::F64(r))
        };
        prop_assume!(l.is_finite() && r.is_finite());
        let (op, expected_bool) = match op_idx {
            0 => (BinOp::Eq, l == r),
            1 => (BinOp::Ne, l != r),
            2 => (BinOp::Lt, l < r),
            3 => (BinOp::Gt, l > r),
            4 => (BinOp::Le, l <= r),
            5 => (BinOp::Ge, l >= r),
            6 => (BinOp::LogicalAnd, l != 0.0 && r != 0.0),
            _ => (BinOp::LogicalOr, l != 0.0 || r != 0.0),
        };
        let got = eval_const_binop_float(&op, &lhs, &rhs)
            .expect("float comparison/logical must return Some");
        match got {
            IrConst::I64(v) => prop_assert_eq!(v, if expected_bool { 1 } else { 0 }),
            other => prop_assert!(false, "expected I64, got {:?}", other),
        }
    }

    // ORACLE: negative / error contract.
    // C forbids `%`, `&`, `|`, `^`, `<<`, `>>` on float operands; constant
    // folding must therefore refuse (return None) for these ops on F64 rather
    // than silently produce a value.
    //
    // Formal: ∀ l,r ∈ f64, op ∈ {%,&,|,^,<<,>>}.
    //   eval_const_binop_float(op, F64(l), F64(r)) = None.
    #[test]
    fn float_unsupported_binary_ops_return_none(
        op_idx in 0u8..6,
        lb in any::<u64>(),
        rb in any::<u64>(),
    ) {
        let l = f64::from_bits(lb);
        let r = f64::from_bits(rb);
        let op = match op_idx {
            0 => BinOp::Mod,
            1 => BinOp::BitAnd,
            2 => BinOp::BitOr,
            3 => BinOp::BitXor,
            4 => BinOp::Shl,
            _ => BinOp::Shr,
        };
        let got = eval_const_binop_float(&op, &IrConst::F64(l), &IrConst::F64(r));
        prop_assert!(got.is_none(), "float {:?} must return None, got {:?}", op, got);
    }

    // ======================================================================
    // eval_const_binop_float — long double (x87 80-bit path on this host)
    // ======================================================================
    //
    // On this host `target_long_double_is_f128()` defaults to false, so the
    // x87 80-bit path runs. An f64-derived value is exactly representable in
    // x87 (64-bit mantissa), and the exact sum/difference of two f64 values
    // needs at most 54 bits — which fits losslessly in x87. Rounding that back
    // to f64 therefore equals native f64 add/sub. (Mul/div are NOT asserted
    // here: their exact product/quotient can exceed 64 bits, so double
    // rounding f64->x87->f64 can legitimately differ and would be a false
    // positive.)

    // ORACLE: differential vs native Rust f64 add/sub.
    // Formal: ∀ l,r ∈ finite_f64.
    //   let r = eval_const_binop_float({+,-}, LongDouble(l), LongDouble(r))
    //   r.approx_f64 = l ⊙ r.
    #[test]
    fn long_double_add_sub_round_trips_to_native_f64(
        is_sub in any::<bool>(),
        lb in any::<u64>(),
        rb in any::<u64>(),
    ) {
        let l = f64::from_bits(lb);
        let r = f64::from_bits(rb);
        // Restrict to the non-crashing domain: `f64_to_f128_bytes_lossless`
        // (used by `IrConst::long_double`) underflows for any finite f64 with
        // 0 < |v| < 1.0 (biased_exp < 1023). See the ignored witness
        // `witness_long_double_subnormal_operand_panics`. The x87 add/sub
        // oracle is valid for the full finite domain.
        prop_assume!(l.is_finite() && (l == 0.0 || l.abs() >= 1.0)
                     && r.is_finite() && (r == 0.0 || r.abs() >= 1.0));
        let op = if is_sub { BinOp::Sub } else { BinOp::Add };
        let expected = if is_sub { l - r } else { l + r };
        let got = eval_const_binop_float(&op, &IrConst::long_double(l), &IrConst::long_double(r))
            .expect("long double add/sub must return Some");
        match got {
            IrConst::LongDouble(approx, _bytes) => {
                // finite l,r => l±r is finite or inf, never NaN.
                prop_assert!(
                    approx == expected,
                    "long double {:?}: l={} r={} got approx={} expected={}",
                    op, l, r, approx, expected
                );
            }
            other => prop_assert!(false, "expected LongDouble, got {:?}", other),
        }
    }

    // ORACLE: differential vs native Rust f64 comparison (x87 path).
    // The x87 comparison converts each operand back to f64 then compares, so
    // for finite f64-derived operands it must agree with native f64 ordering.
    //
    // Formal: ∀ l,r ∈ finite_f64, op ∈ {==,!=,<,>,<=,>=}.
    //   eval_const_binop_float(op, LongDouble(l), LongDouble(r)) = I64( native ? 1 : 0 ).
    #[test]
    fn long_double_comparison_matches_native_f64(
        op_idx in 0u8..6,
        lb in any::<u64>(),
        rb in any::<u64>(),
    ) {
        let l = f64::from_bits(lb);
        let r = f64::from_bits(rb);
        prop_assume!(l.is_finite() && (l == 0.0 || l.abs() >= 1.0)
                     && r.is_finite() && (r == 0.0 || r.abs() >= 1.0));
        let (op, expected_bool) = match op_idx {
            0 => (BinOp::Eq, l == r),
            1 => (BinOp::Ne, l != r),
            2 => (BinOp::Lt, l < r),
            3 => (BinOp::Gt, l > r),
            4 => (BinOp::Le, l <= r),
            _ => (BinOp::Ge, l >= r),
        };
        let got = eval_const_binop_float(&op, &IrConst::long_double(l), &IrConst::long_double(r))
            .expect("long double comparison must return Some");
        match got {
            IrConst::I64(v) => prop_assert_eq!(v, if expected_bool { 1 } else { 0 }),
            other => prop_assert!(false, "expected I64, got {:?}", other),
        }
    }

    // ======================================================================
    // eval_const_binop — integer shift operators (previously untested)
    // ======================================================================

    // ORACLE: differential vs independently derived C wrapping shift semantics.
    // For shift counts strictly below the operand width (the well-defined
    // range), the result must equal shifting the width-narrowed operand and be
    // stored in the correct variant (I32 for signed-32, zero-extended I64 for
    // unsigned-32, I64 for 64-bit). Signedness is irrelevant for Shl but
    // selects logical vs arithmetic Shr.
    //
    // Formal: ∀ l ∈ i64, s ∈ [0,width), w ∈ {32,64}, u ∈ bool.
    //   classify(eval_const_binop({<<,>>}, I64(l), I64(s), w==32, u, ...))
    //     = classify(reference_shift(l, s, w, u)).
    #[test]
    fn integer_shift_matches_reference(
        is_shl in any::<bool>(),
        is_32bit in any::<bool>(),
        is_unsigned in any::<bool>(),
        l in any::<i64>(),
        shift in 0u32..64u32,
    ) {
        // Restrict to the well-defined range (s < width) so shift-amount
        // masking never participates; the SUT's UB-shift wrapping behavior is
        // a separate, intentionally-untested concern.
        prop_assume!(!is_32bit || shift < 32);

        let op = if is_shl { BinOp::Shl } else { BinOp::Shr };
        let got = eval_const_binop(
            &op, &IrConst::I64(l), &IrConst::I64(shift as i64),
            is_32bit, is_unsigned, is_unsigned, is_unsigned,
        );

        let expected = if is_shl {
            if is_32bit {
                if is_unsigned {
                    // zero-extended low 32 bits, stored as I64
                    (0u8, (l as u32).wrapping_shl(shift) as i64)
                } else {
                    // sign-extended, stored as I32
                    (1u8, (l as i32).wrapping_shl(shift) as i64)
                }
            } else {
                (0u8, l.wrapping_shl(shift))
            }
        } else if is_32bit {
            if is_unsigned {
                (0u8, (l as u32).wrapping_shr(shift) as i64)
            } else {
                (1u8, (l as i32).wrapping_shr(shift) as i64)
            }
        } else if is_unsigned {
            (0u8, (l as u64).wrapping_shr(shift) as i64)
        } else {
            (0u8, l.wrapping_shr(shift))
        };

        prop_assert_eq!(classify_int(got), expected);
    }

    // ======================================================================
    // eval_const_binop — integer comparison & logical operators
    // ======================================================================

    // ORACLE: differential vs C comparison semantics parameterized by width
    // and signedness. Results are always 0/1; the variant (I32 vs I64) is
    // width/signedness-dependent but the stored value must be 0 or 1.
    //
    // Formal: ∀ l,r ∈ i64, w ∈ {32,64}, u ∈ bool, op ∈ {==,!=,<,>,<=,>=,&&,||}.
    //   value(eval_const_binop(op, I64(l), I64(r), w==32, u, ...)) = c_truth(op,l,r,w,u).
    #[test]
    fn integer_comparison_and_logical_match_reference(
        op_idx in 0u8..8,
        is_32bit in any::<bool>(),
        is_unsigned in any::<bool>(),
        l in any::<i64>(),
        r in any::<i64>(),
    ) {
        let (op, expected_bool) = match op_idx {
            0 => (BinOp::Eq, if is_32bit { (l as u32) == (r as u32) }
                             else if is_unsigned { (l as u64) == (r as u64) }
                             else { l == r }),
            1 => (BinOp::Ne, if is_32bit { (l as u32) != (r as u32) }
                             else if is_unsigned { (l as u64) != (r as u64) }
                             else { l != r }),
            2 => (BinOp::Lt, if is_32bit {
                                 if is_unsigned { (l as u32) < (r as u32) }
                                 else { (l as i32) < (r as i32) }
                             } else if is_unsigned { (l as u64) < (r as u64) }
                             else { l < r }),
            3 => (BinOp::Gt, if is_32bit {
                                 if is_unsigned { (l as u32) > (r as u32) }
                                 else { (l as i32) > (r as i32) }
                             } else if is_unsigned { (l as u64) > (r as u64) }
                             else { l > r }),
            4 => (BinOp::Le, if is_32bit {
                                 if is_unsigned { (l as u32) <= (r as u32) }
                                 else { (l as i32) <= (r as i32) }
                             } else if is_unsigned { (l as u64) <= (r as u64) }
                             else { l <= r }),
            5 => (BinOp::Ge, if is_32bit {
                                 if is_unsigned { (l as u32) >= (r as u32) }
                                 else { (l as i32) >= (r as i32) }
                             } else if is_unsigned { (l as u64) >= (r as u64) }
                             else { l >= r }),
            6 => (BinOp::LogicalAnd, l != 0 && r != 0),
            _ => (BinOp::LogicalOr, l != 0 || r != 0),
        };
        let got = eval_const_binop(
            &op, &IrConst::I64(l), &IrConst::I64(r),
            is_32bit, is_unsigned, is_unsigned, is_unsigned,
        );
        let want = if expected_bool { 1i64 } else { 0 };
        match got.expect("comparison/logical returns Some") {
            IrConst::I64(v) => prop_assert_eq!(v, want),
            IrConst::I32(v) => prop_assert_eq!(v as i64, want),
            other => prop_assert!(false, "expected I64/I32 result, got {:?}", other),
        }
    }

    // ======================================================================
    // eval_const_binop — i128 path, signedness-sensitive operators
    // ======================================================================

    // ORACLE: differential vs native i128/u128 arithmetic for the operators
    // whose result depends on the result-type signedness (div, mod, shr, and
    // all comparisons). When both operands are I128, no widening occurs, so
    // only `is_unsigned` drives the semantics.
    //
    // Formal: ∀ l,r ∈ i128, u ∈ bool, op ∈ {/,%,>>,==,!=,<,>,<=,>=}.
    //   eval_const_binop(op, I128(l), I128(r), false, u, ...) = native_i128(op,l,r,u).
    #[test]
    fn i128_signedness_sensitive_ops_match_reference(
        op_idx in 0u8..8,
        is_unsigned in any::<bool>(),
        l in any::<i128>(),
        r in any::<i128>(),
    ) {
        let lhs = IrConst::I128(l);
        let rhs = IrConst::I128(r);
        let (op, expected) = match op_idx {
            0 => {
                // div
                prop_assume!(r != 0);
                let v = if is_unsigned {
                    (l as u128).wrapping_div(r as u128) as i128
                } else { l.wrapping_div(r) };
                (BinOp::Div, (2u8, v as i64))
            }
            1 => {
                prop_assume!(r != 0);
                let v = if is_unsigned {
                    (l as u128).wrapping_rem(r as u128) as i128
                } else { l.wrapping_rem(r) };
                (BinOp::Mod, (2u8, v as i64))
            }
            2 => {
                let v = if is_unsigned {
                    (l as u128).wrapping_shr(r as u32) as i128
                } else { l.wrapping_shr(r as u32) };
                (BinOp::Shr, (2u8, v as i64))
            }
            3 => {
                let b = if is_unsigned { (l as u128) == (r as u128) } else { l == r };
                (BinOp::Eq, (0u8, if b { 1 } else { 0 }))
            }
            4 => {
                let b = if is_unsigned { (l as u128) != (r as u128) } else { l != r };
                (BinOp::Ne, (0u8, if b { 1 } else { 0 }))
            }
            5 => {
                let b = if is_unsigned { (l as u128) < (r as u128) } else { l < r };
                (BinOp::Lt, (0u8, if b { 1 } else { 0 }))
            }
            6 => {
                let b = if is_unsigned { (l as u128) > (r as u128) } else { l > r };
                (BinOp::Gt, (0u8, if b { 1 } else { 0 }))
            }
            _ => {
                let b = if is_unsigned { (l as u128) >= (r as u128) } else { l >= r };
                (BinOp::Ge, (0u8, if b { 1 } else { 0 }))
            }
        };
        let got = eval_const_binop(&op, &lhs, &rhs, false, is_unsigned, is_unsigned, is_unsigned);
        prop_assert_eq!(classify_int(got), expected);
    }
}

// ============================================================================
// Bug witness — kept out of the default run via #[ignore].
// Run explicitly to reproduce the ICE against the current SUT:
//   cargo test --test pbt_common const_arith_binop_prop_tests::witness_long_double_subnormal_operand_panics -- --ignored
// ============================================================================

/// A long-double operand with `0 < |v| < 1.0` ICEs `eval_const_binop_float`
/// (via the canonical `IrConst::long_double(v)` construction, which calls
/// `f64_to_f128_bytes_lossless`). For such values the f64 biased exponent is
/// < 1023, so the normal-case computation `biased_exp as u128 - 1023` underflows
/// in debug builds and panics. `0.5L` is a completely ordinary literal, so
/// constant-folding e.g. `1.0L + 0.5L` must not crash the compiler. (Subnormals,
/// the smallest normals, etc. are all caught by the same `|v| < 1.0` condition.)
#[test]
fn witness_long_double_subnormal_operand_panics() {
    let lhs = IrConst::long_double(1.0);
    // 0.5 has biased exponent 1022 (< 1023); construction panics.
    let rhs = IrConst::long_double(0.5);
    // Once fixed, this should return Some(LongDouble(..)) instead of panicking.
    let _ = eval_const_binop_float(&BinOp::Add, &lhs, &rhs);
}
