//! Direct, once-rounded hexadecimal literals for binary32/64/128 and x87.
//!
//! Keep 32 significant hex digits (125–128 bits when full) and jam a nonzero tail into
//! the low bit. All supported formats discard at least 12 bits when the input
//! fills that accumulator, so the jam is strictly below their rounding bit.
//! No floating intermediate, unbounded bigint, or exponent-sized allocation.

fn components(text: &str) -> (u128, i64) {
    let bytes = text.as_bytes();
    let mut i = 2; // caller has recognized the 0x/0X prefix
    let mut mantissa = 0u128;
    let mut kept = 0;
    let mut dropped = 0i64;
    let mut fraction = 0i64;
    let mut after_point = false;
    let mut sticky = false;
    while let Some(&c) = bytes.get(i) {
        if c == b'.' {
            after_point = true;
            i += 1;
            continue;
        }
        let digit = match c {
            b'0'..=b'9' => c - b'0',
            b'a'..=b'f' => c - b'a' + 10,
            b'A'..=b'F' => c - b'A' + 10,
            _ => break,
        };
        if after_point {
            fraction = fraction.saturating_add(1);
        }
        if mantissa != 0 || digit != 0 {
            if kept < 32 {
                mantissa = (mantissa << 4) | u128::from(digit);
                kept += 1;
            } else {
                dropped = dropped.saturating_add(1);
                sticky |= digit != 0;
            }
        }
        i += 1;
    }
    let mut exponent = 0i64;
    if matches!(bytes.get(i), Some(b'p' | b'P')) {
        i += 1;
        let negative = bytes.get(i) == Some(&b'-');
        if matches!(bytes.get(i), Some(b'+' | b'-')) {
            i += 1;
        }
        while let Some(&c) = bytes.get(i) {
            if !c.is_ascii_digit() {
                break;
            }
            exponent = exponent
                .saturating_mul(10)
                .saturating_add(i64::from(c - b'0'));
            i += 1;
        }
        if negative {
            exponent = -exponent;
        }
    }
    let exponent = exponent
        .saturating_sub(fraction.saturating_mul(4))
        .saturating_add(dropped.saturating_mul(4));
    (mantissa | u128::from(sticky), exponent)
}

fn bits(text: &str, negative: bool, fraction_bits: u32, exponent_bits: u32) -> u128 {
    let sign = u128::from(negative) << (fraction_bits + exponent_bits);
    let (mantissa, scale) = components(text);
    if mantissa == 0 {
        return sign;
    }
    let high = 127 - mantissa.leading_zeros();
    let mut exponent = scale.saturating_add(i64::from(high));
    let bias = (1i64 << (exponent_bits - 1)) - 1;
    let min_exponent = 1 - bias;
    let infinity = sign | (((1u128 << exponent_bits) - 1) << fraction_bits);
    if exponent > bias {
        return infinity;
    }
    if exponent < min_exponent - i64::from(fraction_bits) - 1 {
        return sign;
    }
    let shift = i64::from(high) - i64::from(fraction_bits) + (min_exponent - exponent).max(0);
    let mut rounded = if shift > 0 {
        let (truncated, up) = super::long_double::round_shift_half_even(mantissa, shift);
        truncated + u128::from(up)
    } else {
        mantissa << (-shift as u32)
    };
    if exponent < min_exponent {
        // Carry to bit fraction_bits is exactly the encoding of min-normal.
        return sign | rounded;
    }
    if rounded == 1u128 << (fraction_bits + 1) {
        rounded >>= 1;
        exponent += 1;
    }
    if exponent > bias {
        infinity
    } else {
        sign | (((exponent + bias) as u128) << fraction_bits)
            | (rounded & ((1u128 << fraction_bits) - 1))
    }
}

pub(crate) fn parse_f64(text: &str, single: bool) -> f64 {
    if single {
        f32::from_bits(bits(text, false, 23, 8) as u32) as f64
    } else {
        f64::from_bits(bits(text, false, 52, 11) as u64)
    }
}

/// Round to the target long-double precision BEFORE using the binary128
/// token carrier. Rounding to binary128 and then x87 can double-round values
/// just above an x87 midpoint. The precision-64 result widens exactly by 49
/// fraction bits; both formats have the same exponent bias and range.
pub(crate) fn parse_long_double(text: &str, is_f128: bool) -> [u8; 16] {
    if is_f128 {
        parse_f128(text, false)
    } else {
        (bits(text, false, 63, 15) << 49).to_le_bytes()
    }
}

pub(crate) fn parse_f128(text: &str, negative: bool) -> [u8; 16] {
    bits(text, negative, 112, 15).to_le_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_ieee_subnormal_normal_overflow_boundaries() {
        for (text, want) in [
            ("0x1p-1074", 1),
            ("0x1p-1075", 0),
            ("0x3p-1075", 2),
            ("0x1.fffffffffffffp-1023", 1 << 52),
            ("0x1.fffffffffffffp1023", 0x7fefffffffffffff),
            ("0x1.fffffffffffff8p1023", 0x7ff0000000000000),
            ("0x100000000000000000000p-80", 0x3ff0000000000000),
            ("0x000000000000000000000000000000000000000001p-1074", 1),
            ("0x1p999999999999999999999", 0x7ff0000000000000),
            ("0x1p-999999999999999999999", 0),
            ("0x0p999999999999999999999", 0),
        ] {
            assert_eq!(parse_f64(text, false).to_bits(), want, "{text}");
        }
    }

    #[test]
    fn hex_ieee_ties_and_arbitrarily_distant_sticky_bits() {
        for (text, want) in [
            (
                "0x1.00000000000008000000000000000000000000000p0",
                0x3ff0000000000000,
            ),
            (
                "0x1.00000000000008000000000000000000000000001p0",
                0x3ff0000000000001,
            ),
            (
                "0x1.00000000000018000000000000000000000000000p0",
                0x3ff0000000000002,
            ),
            ("0x1.00000000000000000000000000000000000000001p-1075", 1),
        ] {
            assert_eq!(parse_f64(text, false).to_bits(), want, "{text}");
        }
    }

    #[test]
    fn hex_ieee_binary32_is_not_double_rounded() {
        for (text, want) in [
            ("0x1p-149", 1),
            ("0x1p-150", 0),
            ("0x3p-150", 2),
            ("0x1.000001p0", 0x3f800000),
            ("0x1.000001000000000000000000001p0", 0x3f800001),
            ("0x1.fffffep127", 0x7f7fffff),
            ("0x1.ffffffp127", 0x7f800000),
        ] {
            assert_eq!((parse_f64(text, true) as f32).to_bits(), want, "{text}");
        }
    }

    #[test]
    fn hex_x87_rounds_before_widening_to_the_token_carrier() {
        let midpoint = format!("0x1.{}1{}p0", "0".repeat(15), "0".repeat(40));
        let above = midpoint.replace("p0", "1p0");
        let one = 16383u128 << 112;
        assert_eq!(
            u128::from_le_bytes(parse_long_double(&midpoint, false)),
            one
        );
        assert_eq!(
            u128::from_le_bytes(parse_long_double(&above, false)),
            one + (1 << 49)
        );
        // Quad targets and explicit Q suffixes must retain their own precision.
        assert_eq!(parse_long_double(&above, true), parse_f128(&above, false));
        assert_ne!(
            parse_long_double(&above, true),
            parse_long_double(&above, false)
        );
        for (literal, mantissa, exponent) in [
            ("0x1p-16445", 1u64, 0u16),
            ("0x1p-16446", 0, 0),
            ("0x3p-16446", 2, 0),
            ("0x1.fffffffffffffffep16383", u64::MAX, 0x7ffe),
            ("0x1.ffffffffffffffffp16383", 1 << 63, 0x7fff),
        ] {
            let wide = parse_long_double(literal, false);
            let x87 = super::super::long_double::f128_bytes_to_x87_bytes(&wide);
            assert_eq!(
                u64::from_le_bytes(x87[..8].try_into().unwrap()),
                mantissa,
                "{literal}"
            );
            assert_eq!(
                u16::from_le_bytes(x87[8..10].try_into().unwrap()),
                exponent,
                "{literal}"
            );
        }
    }

    #[test]
    fn hex_ieee_binary128_retains_tail_and_rounds_subnormals_once() {
        assert_eq!(u128::from_le_bytes(parse_f128("0x1p-16494", false)), 1);
        assert_eq!(u128::from_le_bytes(parse_f128("0x3p-16495", false)), 2);
        assert_eq!(
            u128::from_le_bytes(parse_f128("0x1p-16495", true)),
            1 << 127
        );
        let one = 16383u128 << 112;
        let midpoint = format!("0x1.{}8{}p0", "0".repeat(28), "0".repeat(50));
        let above = midpoint.replace("p0", "1p0");
        assert_eq!(u128::from_le_bytes(parse_f128(&midpoint, false)), one);
        assert_eq!(u128::from_le_bytes(parse_f128(&above, false)), one + 1);
    }
}
