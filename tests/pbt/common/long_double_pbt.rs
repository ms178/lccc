// Oracle: Algebraic — Round-trip (4a)
// f128/x87 have MORE precision than f64, so converting f64 → f128/x87 → f64
// must return the exact original for all finite f64 values.
// Manual PBT tests — examples for pi-pbt to learn from.

mod pbt_long_double_tests {
    use lccc::pbt_internals::{
        f64_to_f128_bytes_lossless, f64_to_x87_bytes_simple, f128_bytes_to_f64, x87_bytes_to_f64,
    };
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn f64_to_f128_roundtrip(val in any::<f64>().prop_filter("finite", |v| v.is_finite())) {
            let bytes = f64_to_f128_bytes_lossless(val);
            let back = f128_bytes_to_f64(&bytes);
            prop_assert_eq!(val.to_bits(), back.to_bits(),
                "f64→f128→f64 round-trip failed: {} → {:?} → {}", val, bytes, back);
        }

        #[test]
        fn f64_to_x87_roundtrip(val in any::<f64>().prop_filter("finite", |v| v.is_finite())) {
            let bytes16 = f64_to_x87_bytes_simple(val);
            let back = x87_bytes_to_f64(&bytes16);
            prop_assert_eq!(val.to_bits(), back.to_bits(),
                "f64→x87→f64 round-trip failed: {} → {:?} → {}", val, bytes16, back);
        }

        #[test]
        fn f64_zero_sign_preserved_in_f128(sign in any::<bool>()) {
            let val = if sign { -0.0f64 } else { 0.0f64 };
            let bytes = f64_to_f128_bytes_lossless(val);
            let back = f128_bytes_to_f64(&bytes);
            prop_assert_eq!(val.to_bits(), back.to_bits(),
                "zero sign not preserved: input bits={:#018x} output bits={:#018x}",
                val.to_bits(), back.to_bits());
        }

        #[test]
        fn f64_special_values_survive_f128_roundtrip(val in prop_oneof![
            Just(f64::INFINITY),
            Just(f64::NEG_INFINITY),
            Just(f64::NAN),
        ]) {
            let bytes = f64_to_f128_bytes_lossless(val);
            let back = f128_bytes_to_f64(&bytes);
            if val.is_nan() {
                prop_assert!(back.is_nan(), "NaN did not survive: got {}", back);
            } else {
                prop_assert_eq!(val.to_bits(), back.to_bits(),
                    "special value changed: {} → {}", val, back);
            }
        }
    }
}
