//! Exact IEEE-754 binary16 (`_Float16` / `__fp16`) and brain-float16 (`__bf16`)
//! bit-pattern conversions, arithmetic helpers, and NaN payload constructors.
//!
//! All narrowing conversions (`f32 -> f16`, `f64 -> f16`, `f32 -> bf16`,
//! `f64 -> bf16`, `i128/u128 -> f16/bf16`) perform single-step IEEE-754
//! round-to-nearest-ties-to-even (RNE) with zero intermediate double-rounding,
//! matching GCC 16.2 (`libgcc` / MPFR) bit-for-bit including quiet-NaN bit
//! propagation.

/// Widen an IEEE-754 binary16 (`_Float16`) 16-bit pattern to `f32` (lossless,
/// quieting any signaling NaN bit to match `__extendhfsf2` / `vcvtph2ps`).
#[inline]
pub(crate) fn f16_bits_to_f32(bits: u16) -> f32 {
    let sign = ((bits as u32) & 0x8000) << 16;
    let exp = ((bits >> 10) & 0x1f) as u32;
    let mant = (bits & 0x03ff) as u32;
    let f32_bits = if exp == 0 {
        if mant == 0 {
            sign
        } else {
            let lz = (mant as u16).leading_zeros() - 6;
            let shift = lz + 1;
            let norm_mant = (mant << shift) & 0x03ff;
            let f32_exp = (127u32 - 15u32) - lz;
            sign | (f32_exp << 23) | (norm_mant << 13)
        }
    } else if exp == 0x1f {
        if mant == 0 {
            sign | 0x7f80_0000
        } else {
            sign | 0x7fc0_0000 | (mant << 13)
        }
    } else {
        let f32_exp = exp + (127 - 15);
        sign | (f32_exp << 23) | (mant << 13)
    };
    f32::from_bits(f32_bits)
}

/// Widen an IEEE-754 binary16 (`_Float16`) 16-bit pattern to `f64`.
#[inline]
pub(crate) fn f16_bits_to_f64(bits: u16) -> f64 {
    f16_bits_to_f32(bits) as f64
}

/// Convert `f64` to `f32` under round-to-odd (Boldo-Melquiond sticky jamming),
/// guaranteeing that a subsequent RNE `f32 -> f16` or `f32 -> bf16` conversion
/// produces the exact single-rounded IEEE-754 RNE result with zero double-rounding.
#[inline]
pub(crate) fn f64_to_f32_round_to_odd(val: f64) -> f32 {
    let bits = val.to_bits();
    let sign = ((bits >> 63) as u32) << 31;
    let exp = ((bits >> 52) & 0x7ff) as i32;
    let mant = bits & 0x000f_ffff_ffff_ffff;
    if exp == 0x7ff {
        if mant == 0 {
            return f32::from_bits(sign | 0x7f80_0000);
        }
        let payload = ((mant >> 29) as u32) & 0x003f_ffff;
        return f32::from_bits(sign | 0x7fc0_0000 | payload);
    }
    if exp == 0 && mant == 0 {
        return f32::from_bits(sign);
    }
    let unb = exp - 1023;
    if unb > 127 {
        // Clamp to max finite odd f32 (still overflows to Inf in both f16 and bf16,
        // wait: bf16 max exponent IS 127! Let's handle >127 exact RNE directly in callers
        // or return +/-Inf only when unb > 128!).
        return f32::from_bits(sign | 0x7f80_0000);
    }
    if unb < -149 {
        return f32::from_bits(sign | 1);
    }
    let sig53 = if exp == 0 {
        mant << 1
    } else {
        mant | (1u64 << 52)
    };
    if unb >= -126 {
        let f32_exp = ((unb + 127) as u32) << 23;
        let top24 = ((sig53 >> 29) as u32) & 0x007f_ffff;
        let sticky = u32::from((sig53 & 0x1fff_ffff) != 0);
        f32::from_bits(sign | f32_exp | (top24 | sticky))
    } else {
        let shift = (29 + (-126 - unb)) as u32;
        let main = if shift >= 64 {
            0
        } else {
            (sig53 >> shift) as u32
        };
        let rem_mask = if shift >= 64 {
            u64::MAX
        } else {
            (1u64 << shift) - 1
        };
        let sticky = u32::from((sig53 & rem_mask) != 0);
        f32::from_bits(sign | (main | sticky))
    }
}

/// Convert `f32` to IEEE-754 binary16 (`_Float16`) under RNE.
#[inline]
pub(crate) fn f32_to_f16_bits(val: f32) -> u16 {
    let bits = val.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exp = ((bits >> 23) & 0xff) as i32;
    let mant = bits & 0x007f_ffff;

    if exp == 0xff {
        if mant == 0 {
            return sign | 0x7c00;
        }
        let payload = ((mant >> 13) as u16) & 0x03ff;
        return sign | 0x7e00 | payload;
    }

    let unb = exp - 127;
    if unb > 15 {
        return sign | 0x7c00;
    }
    if unb < -25 {
        return sign;
    }

    let sig = mant | 0x0080_0000;
    if unb < -14 {
        // Subnormal f16 (or underflow to zero / round up to min normal 0x0400).
        // Shift amount from 24-bit `sig` (with binary point after bit 23) to
        // 10-bit subnormal unit `2^-24` is `(-14 - unb) + 13 = -1 - unb`.
        let shift = (-1 - unb) as u32;
        let main = if shift >= 32 { 0 } else { sig >> shift };
        let half = 1u32 << (shift - 1);
        let rem = sig & ((1u32 << shift) - 1);
        let round_up = rem > half || (rem == half && (main & 1) != 0);
        let res = main + u32::from(round_up);
        return sign | (res as u16);
    }

    let f16_exp = ((unb + 15) as u16) << 10;
    let main = (mant >> 13) as u16;
    let rem = mant & 0x1fff;
    let round_up = rem > 0x1000 || (rem == 0x1000 && (main & 1) != 0);
    // Note: if `main == 0x03ff` and `round_up`, adding 1 carries cleanly into
    // `f16_exp` (including `exp=30 -> 31` = +Inf `0x7c00`).
    sign | (f16_exp + main + u16::from(round_up))
}

/// Convert `f64` to IEEE-754 binary16 (`_Float16`) under single-step RNE.
#[inline]
pub(crate) fn f64_to_f16_bits(val: f64) -> u16 {
    f32_to_f16_bits(f64_to_f32_round_to_odd(val))
}

/// Widen a `__bf16` 16-bit pattern to `f32` (matching GCC `__extendbfsf2`,
/// including SNaN quieting).
#[inline]
pub(crate) fn bf16_bits_to_f32(bits: u16) -> f32 {
    let quieted = if (bits & 0x7fff) > 0x7f80 {
        bits | 0x0040
    } else {
        bits
    };
    f32::from_bits((quieted as u32) << 16)
}

/// Widen a `__bf16` 16-bit pattern to `f64`.
#[inline]
pub(crate) fn bf16_bits_to_f64(bits: u16) -> f64 {
    bf16_bits_to_f32(bits) as f64
}

/// Convert `f32` to `__bf16` under IEEE-754 RNE (matching GCC `__truncsfbf2`).
#[inline]
pub(crate) fn f32_to_bf16_bits(val: f32) -> u16 {
    let bits = val.to_bits();
    if (bits & 0x7fff_ffff) > 0x7f80_0000 {
        return ((bits >> 16) as u16) | 0x0040;
    }
    let bias = 0x7fff_u32 + ((bits >> 16) & 1);
    (bits.wrapping_add(bias) >> 16) as u16
}

/// Convert `f64` to `__bf16` under single-step IEEE-754 RNE (no double-rounding).
#[inline]
pub(crate) fn f64_to_bf16_bits(val: f64) -> u16 {
    let bits = val.to_bits();
    let sign = ((bits >> 48) & 0x8000) as u16;
    let exp = ((bits >> 52) & 0x7ff) as i32;
    let mant = bits & 0x000f_ffff_ffff_ffff;
    if exp == 0x7ff {
        if mant == 0 {
            return sign | 0x7f80;
        }
        let payload = ((mant >> 45) as u16) & 0x007f;
        return sign | 0x7fc0 | payload;
    }
    // Check boundary near `2^128 * (1 - 2^-9)` where `f64_to_f32_round_to_odd`
    // would overflow `f32` (`2^128 * (1 - 2^-25)`) before `bf16` (`2^128 * (1 - 2^-9)`):
    let unb = exp - 1023;
    if unb == 127 {
        let bf_mant = ((mant >> 45) & 0x7f) as u16;
        let rem = mant & ((1u64 << 45) - 1);
        let half = 1u64 << 44;
        let round_up = rem > half || (rem == half && (bf_mant & 1) != 0);
        return sign | ((0xfe_u16 << 7) + bf_mant + u16::from(round_up));
    }
    f32_to_bf16_bits(f64_to_f32_round_to_odd(val))
}

/// Convert signed `i128` to `_Float16` bits under single-step RNE.
#[inline]
pub(crate) fn i128_to_f16_bits(val: i128) -> u16 {
    // Every `i128` value outside `[-65520, 65520]` overflows `_Float16` to `±Inf`,
    // and every integer in `[-65520, 65520]` is 100% exact in `f64` (and `f32`)!
    let clamped = val.clamp(-131_072, 131_072) as f64;
    f64_to_f16_bits(clamped)
}

/// Convert unsigned `u128` to `_Float16` bits under single-step RNE.
#[inline]
pub(crate) fn u128_to_f16_bits(val: u128) -> u16 {
    let clamped = val.min(131_072) as f64;
    f64_to_f16_bits(clamped)
}

/// Convert signed `i128` to `__bf16` bits under single-step RNE.
#[inline]
pub(crate) fn i128_to_bf16_bits(val: i128) -> u16 {
    if val == 0 {
        return 0;
    }
    let sign = if val < 0 { 0x8000u16 } else { 0u16 };
    let mag = val.unsigned_abs();
    sign | u128_to_bf16_bits(mag)
}

/// Convert unsigned `u128` to `__bf16` bits under single-step RNE.
#[inline]
pub(crate) fn u128_to_bf16_bits(val: u128) -> u16 {
    if val == 0 {
        return 0;
    }
    let bit_len = 128 - val.leading_zeros();
    let unb = (bit_len - 1) as u32;
    if bit_len <= 8 {
        let mant = ((val << (8 - bit_len)) & 0x7f) as u16;
        return (((unb + 127) as u16) << 7) | mant;
    }
    let shift = bit_len - 8;
    let main = ((val >> shift) & 0x7f) as u16;
    let half = 1u128 << (shift - 1);
    let rem = val & ((1u128 << shift) - 1);
    let round_up = rem > half || (rem == half && (main & 1) != 0);
    // For `u128::MAX` (`unb == 127`), `0xfe << 7 + 0x7f + 1 = 0x7f80` (+Inf)!
    (((unb + 127) as u16) << 7) + main + u16::from(round_up)
}

/// Parse a C `nan(...)` payload string (decimal, `0x` hex, or `0` octal).
fn parse_nan_payload(s: &str) -> Option<u64> {
    let s = s.trim();
    if s.is_empty() {
        return Some(0);
    }
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u64::from_str_radix(hex, 16).ok()
    } else if let Some(oct) = s.strip_prefix('0') {
        if oct.is_empty() {
            Some(0)
        } else {
            u64::from_str_radix(oct, 8).ok()
        }
    } else {
        s.parse::<u64>().ok()
    }
}

/// Construct `__builtin_nanf16` (`signaling = false`) or `__builtin_nansf16`
/// (`signaling = true`) bit pattern matching GCC 16.2.
pub(crate) fn nanf16_bits(payload_str: &str, signaling: bool) -> u16 {
    let payload = (parse_nan_payload(payload_str).unwrap_or(0) as u16) & 0x01ff;
    if signaling {
        let sig_payload = if payload == 0 { 0x0100 } else { payload };
        0x7c00 | sig_payload
    } else {
        0x7e00 | payload
    }
}

/// Construct `__builtin_nansf16b` (`signaling = true`) or quiet bf16 NaN bit
/// pattern matching GCC 16.2 (`0x7fa0` for `""`, `0x7f81` for `"1"`).
pub(crate) fn nanbf16_bits(payload_str: &str, signaling: bool) -> u16 {
    let payload = (parse_nan_payload(payload_str).unwrap_or(0) as u16) & 0x003f;
    if signaling {
        let sig_payload = if payload == 0 { 0x0020 } else { payload };
        0x7f80 | sig_payload
    } else {
        0x7fc0 | payload
    }
}

/// Exact IEEE-754 binary16 `nextafterf16(x, y)` on raw 16-bit patterns.
pub(crate) fn nextafterf16_bits(x: u16, y: u16) -> u16 {
    let ax = x & 0x7fff;
    let ay = y & 0x7fff;
    if ax > 0x7c00 {
        return x | 0x0200;
    }
    if ay > 0x7c00 {
        return y | 0x0200;
    }
    if ax == 0 && ay == 0 {
        return y;
    }
    if x == y {
        return y;
    }
    if ax == 0 {
        return (y & 0x8000) | 0x0001;
    }
    let sx = x >> 15;
    let sy = y >> 15;
    let x_lt_y = if sx != sy {
        sx == 1
    } else if sx == 0 {
        ax < ay
    } else {
        ax > ay
    };
    let inc_mag = (sx == 0 && x_lt_y) || (sx == 1 && !x_lt_y);
    if inc_mag {
        x.wrapping_add(1)
    } else {
        let dec = x.wrapping_sub(1);
        if (dec & 0x7fff) == 0 { sx << 15 } else { dec }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exhaustive_f16_roundtrip_all_65536_patterns() {
        for bits in 0u16..=u16::MAX {
            let f = f16_bits_to_f32(bits);
            let back = f32_to_f16_bits(f);
            let exp = (bits >> 10) & 0x1f;
            let mant = bits & 0x03ff;
            if exp == 0x1f && mant != 0 {
                // Any NaN round-trips to the quieted NaN with identical sign & low payload.
                assert_eq!(back, bits | 0x0200, "NaN roundtrip for {bits:#06x}");
            } else {
                assert_eq!(back, bits, "exact roundtrip failed for {bits:#06x} ({f})");
            }
        }
    }

    #[test]
    fn exhaustive_bf16_roundtrip_all_65536_patterns() {
        for bits in 0u16..=u16::MAX {
            let f = bf16_bits_to_f32(bits);
            let back = f32_to_bf16_bits(f);
            if (bits & 0x7fff) > 0x7f80 {
                assert_eq!(back, bits | 0x0040, "bf16 NaN roundtrip for {bits:#06x}");
            } else {
                assert_eq!(back, bits, "bf16 roundtrip failed for {bits:#06x}");
            }
        }
    }

    #[test]
    fn gcc16_nan_and_double_rounding_oracle_vectors() {
        assert_eq!(nanf16_bits("", false), 0x7e00);
        assert_eq!(nanf16_bits("", true), 0x7d00);
        assert_eq!(nanbf16_bits("", true), 0x7fa0);
        assert_eq!(nanf16_bits("1", false), 0x7e01);
        assert_eq!(nanf16_bits("1", true), 0x7c01);
        assert_eq!(nanbf16_bits("1", true), 0x7f81);
        assert_eq!(
            bf16_bits_to_f32(0x7f81).to_bits(),
            0x7fc1_0000,
            "SNaN quieting matches __extendbfsf2"
        );
        // Classic f64 -> f16 double-rounding trap at odd f16 significand 0x3c01 (1.0 + 2^-10):
        // Value = 1.0 + 2^-10 + 2^-11 - 2^-25 (strictly below the 0x3c01..0x3c02 halfway point!).
        // Naive f64 -> f32 rounds UP to 1.0 + 2^-10 + 2^-11, which RNE tie-breaks UP to 0x3c02.
        // Exact single-step RNE MUST stay at 0x3c01!
        let trap_f64 =
            1.0_f64 + f64::from_bits((1023 - 10) << 52) + f64::from_bits((1023 - 11) << 52)
                - f64::from_bits((1023 - 25) << 52);
        assert_eq!(
            f32_to_f16_bits(trap_f64 as f32),
            0x3c02,
            "naive double-rounds"
        );
        assert_eq!(
            f64_to_f16_bits(trap_f64),
            0x3c01,
            "single-step RNE stays 0x3c01"
        );
    }
}
