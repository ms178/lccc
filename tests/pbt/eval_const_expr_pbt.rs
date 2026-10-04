// Oracle: Algebraic/Reference properties for eval_const_expr.
// Targets the C preprocessor constant expression evaluator.
// Manual PBT — fast, direct, no ceremony.

mod pbt_eval_const_expr_tests {
    use lccc::pbt_internals::eval_const_expr;
    use proptest::prelude::*;

    // Oracle: Algebraic — Metamorphic (4a)
    // Wrapping in parentheses and whitespace must not change the result.
    proptest! {
        #[test]
        fn parentheses_invariant(base in prop_oneof![
            Just("0"), Just("1"), Just("42"), Just("0xFF"),
            Just("1+1"), Just("3*4"), Just("1||0"), Just("1&&0"),
            Just("!0"), Just("~0"), Just("1?2:3"), Just("0?2:3"),
            Just("1<<2"), Just("100>>1"), Just("5%3"), Just("10/2"),
            Just("1==1"), Just("1!=2"), Just("3>2"), Just("2<3"),
            Just("'a'"), Just("'\\n'"), Just("0x7FFFFFFF"),
        ]) {
            let wrapped = format!("  ( {} ) ", base);
            prop_assert_eq!(eval_const_expr(&wrapped), eval_const_expr(base),
                "paren/whitespace changed result for: {}", base);
        }
    }

    // Oracle: Reference (4c)
    // Bare identifiers (not "true"/"false") must evaluate to false (C standard: undefined → 0).
    proptest! {
        #[test]
        fn bare_identifiers_are_false(
            name in "[a-zA-Z_][a-zA-Z0-9_]{0,7}"
                .prop_filter("not true/false", |s| s != "true" && s != "false")
        ) {
            prop_assert!(!eval_const_expr(&name),
                "bare identifier '{}' should be false (undefined → 0)", name);
        }
    }

    // Oracle: Reference (4c)
    // Ternary: "1 ? A : B" must yield eval(A), "0 ? A : B" must yield eval(B).
    proptest! {
        #[test]
        fn ternary_selects_correct_branch(
            a in prop_oneof![Just("1"), Just("42"), Just("0xFF")],
            b in prop_oneof![Just("0"), Just("1"), Just("99")],
        ) {
            let true_branch = format!("1 ? {} : {}", a, b);
            let false_branch = format!("0 ? {} : {}", a, b);
            prop_assert_eq!(eval_const_expr(&true_branch), eval_const_expr(a),
                "1?A:B should yield eval(A) for: {}", true_branch);
            prop_assert_eq!(eval_const_expr(&false_branch), eval_const_expr(b),
                "0?A:B should yield eval(B) for: {}", false_branch);
        }
    }

    // Oracle: Reference (4c)
    // Integer literals: non-zero → true, zero → false.
    proptest! {
        #[test]
        fn nonzero_literals_are_true(val in 1i64..=i64::MAX) {
            let expr = format!("{}", val);
            prop_assert!(eval_const_expr(&expr),
                "non-zero literal '{}' should be true", expr);
        }

        #[test]
        fn zero_literals_are_false(expr in prop_oneof![
            Just("0"), Just("0x0"), Just("00"), Just("0U"), Just("0L"),
            Just("0UL"), Just("0LL"), Just("0ULL"),
        ]) {
            prop_assert!(!eval_const_expr(expr),
                "'{}' should be false", expr);
        }
    }

    // Oracle: Algebraic — Double negation (involution)
    // !!expr should have the same truth value as expr (for well-formed exprs).
    proptest! {
        #[test]
        fn double_negation_preserves_truth(base in prop_oneof![
            Just("0"), Just("1"), Just("42"), Just("0xFF"),
            Just("1+1"), Just("3>2"), Just("0==0"), Just("1&&1"),
        ]) {
            let double_neg = format!("!!({})", base);
            prop_assert_eq!(eval_const_expr(&double_neg), eval_const_expr(base),
                "!!({}) should equal eval({})", base, base);
        }
    }

    // Oracle: Negative/Error Contract (4e)
    // Division by zero must not panic — should return false or 0.
    proptest! {
        #[test]
        fn division_by_zero_does_not_panic(
            lhs in prop_oneof![Just("1"), Just("42"), Just("0"), Just("-1"), Just("0x7FFFFFFF")],
        ) {
            let div = format!("{}/0", lhs);
            let modulo = format!("{}%0", lhs);
            // Must not panic — result is unspecified but must not crash.
            let _ = eval_const_expr(&div);
            let _ = eval_const_expr(&modulo);
        }
    }

    // Oracle: Negative/Error Contract (4e)
    // Shift by negative or >= 64 must not panic.
    proptest! {
        #[test]
        fn extreme_shifts_do_not_panic(
            shift in prop_oneof![Just("64"), Just("65"), Just("128"), Just("-1")],
        ) {
            let left = format!("1<<{}", shift);
            let right = format!("1>>{}", shift);
            let _ = eval_const_expr(&left);
            let _ = eval_const_expr(&right);
        }
    }

    // Oracle: Negative/Error Contract — INT64_MIN negation must not panic.
    #[test]
    fn negation_of_int_min_does_not_panic() {
        // -(-9223372036854775807 - 1) = -INT64_MIN = overflow in signed arithmetic
        let _ = eval_const_expr("-(-9223372036854775807-1)");
        let _ = eval_const_expr("-9223372036854775808");
        let _ = eval_const_expr("-(0x7FFFFFFFFFFFFFFF)");
        let _ = eval_const_expr("-0x8000000000000000");
    }
}
