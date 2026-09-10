//! Compile-time magic numbers for 32-bit division by constants.
//!
//! Uses Hacker's Delight / Granlund–Montgomery-style integer arithmetic
//! (HD section 10, "Integer Division by Constants"). The i686 emitter uses
//! the results with the high half of a 32-bit unsigned/signed multiply
//! (`mull`/`imull`); the IR-level `div_by_const` pass cannot run on i686
//! (it emits 64-bit mulhi sequences the backend truncates), so codegen
//! folds constant division itself.
//!
//! These functions compute arithmetic parameters, not instruction
//! schedules or profitability decisions. The fixed magic values in the
//! tests are regression pins for the immediates GNU/LLVM/ICX emit for
//! `-m32 -O2` (all three derive the same values from the HD algorithm);
//! they are evidence of parameter correctness, not of codegen
//! superiority.
//!
//! # Postconditions are asserted, not merely documented
//!
//! The emitter shifts by `s` (plain form) or `s - 1` (add form) without
//! clamping. An out-of-range shift would encode an invalid or masked
//! immediate and silently miscompile, so the shift-range postconditions
//! are `assert!`ed even in release builds. The same goes for the input
//! preconditions: these run once per compile-time divisor, which is noise
//! next to string emission.

/// Unsigned division magic. Returns `(mul, shift, add)`.
///
/// Let `hi = mulhu(n, mul)`, the high 32 bits of the unsigned product.
///
/// - Without correction: `q = hi >> shift`.
/// - With correction:
///   `q = (hi + ((n - hi) >> 1)) >> (shift - 1)`.
///
/// The addition form has an encoded shift in `1..=32`; its actual final
/// machine shift is therefore in `0..=31`.
///
/// # Panics
///
/// Panics unless `d >= 2` and `d` is not a power of two. Zero, one, and
/// powers of two must be handled by the caller.
pub(super) fn magic_u32(d: u32) -> (u32, u32, bool) {
    assert!(
        d >= 2 && !d.is_power_of_two(),
        "magic_u32 requires a non-power-of-two divisor >= 2"
    );

    let d64 = u64::from(d);
    let all_ones = u64::from(u32::MAX);
    let half = 1u64 << 31;

    // Equivalent to -1 - (-d % d) with the negation evaluated in u32.
    // Widen first so the implementation itself needs no wrapping arithmetic.
    let nc = all_ones - (all_ones - d64 + 1) % d64;

    let mut p = 31u32;
    let mut q1 = half / nc;
    let mut r1 = half - q1 * nc;
    let mut q2 = (half - 1) / d64;
    let mut r2 = (half - 1) - q2 * d64;
    let mut add = false;

    loop {
        p += 1;

        if r1 >= nc - r1 {
            q1 = q1 * 2 + 1;
            r1 = r1 * 2 - nc;
        } else {
            q1 *= 2;
            r1 *= 2;
        }

        if r2 + 1 >= d64 - r2 {
            if q2 >= half - 1 {
                add = true;
            }
            q2 = q2 * 2 + 1;
            r2 = r2 * 2 + 1 - d64;
        } else {
            if q2 >= half {
                add = true;
            }
            q2 *= 2;
            r2 = r2 * 2 + 1;
        }

        let delta = d64 - 1 - r2;

        // For valid u32 divisors the arithmetic termination condition is
        // reached by p == 64. At that point q1 is greater than delta.
        debug_assert!(p <= 64);
        if q1 > delta || (q1 == delta && r1 != 0) {
            break;
        }
    }

    let shift = p - 32;
    assert!(
        if add {
            (1..=32).contains(&shift)
        } else {
            shift < 32
        },
        "magic_u32 produced an out-of-range shift {shift} (add={add})"
    );

    // The low 32 bits are intentional. The add flag accounts for the
    // extra bit when the multiplier requires the correction form.
    ((q2 + 1) as u32, shift, add)
}

/// Signed division magic for a positive divisor `d >= 2`.
///
/// Returns `(mul, shift)`. All operations below on the quotient are i32
/// machine operations, with wrapping addition/subtraction:
///
/// ```text
/// hi = mulhs(n, mul)
/// adjusted = if mul < 0 { hi + n } else { hi }
/// q = (adjusted >> shift) - (n >> 31)
/// ```
///
/// Both right shifts are arithmetic. The final correction implements
/// truncation toward zero.
///
/// For a negative divisor whose magnitude is representable as a positive
/// i32, the caller may compute division by the magnitude and negate the
/// quotient.
///
/// Divisors 0, 1, -1, and i32::MIN require separate handling. In
/// particular, `i32::MIN.unsigned_abs() as i32` is not a valid argument
/// here.
///
/// # Panics
///
/// Panics unless `d >= 2`.
pub(super) fn magic_s32(d: i32) -> (i32, u32) {
    assert!(d >= 2, "magic_s32 requires a positive divisor >= 2");

    let ad = d as u64;
    let two31 = 1u64 << 31;
    let anc = two31 - 1 - two31 % ad;

    let mut p = 31u32;
    let mut q1 = two31 / anc;
    let mut r1 = two31 - q1 * anc;
    let mut q2 = two31 / ad;
    let mut r2 = two31 - q2 * ad;

    loop {
        p += 1;

        q1 *= 2;
        r1 *= 2;
        if r1 >= anc {
            q1 += 1;
            r1 -= anc;
        }

        q2 *= 2;
        r2 *= 2;
        if r2 >= ad {
            q2 += 1;
            r2 -= ad;
        }

        let delta = ad - r2;

        debug_assert!(p <= 63);
        if q1 > delta || (q1 == delta && r1 != 0) {
            break;
        }
    }

    let shift = p - 32;
    assert!(
        shift < 32,
        "magic_s32 produced an out-of-range shift {shift}"
    );

    ((q2 + 1) as i32, shift)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Model the emitter's 32-bit multiply-high and correction operations.
    // These helpers test the magic-number contract, not the actual emitter.
    fn magic_udiv(n: u32, magic: (u32, u32, bool)) -> u32 {
        let (m, s, add) = magic;
        let hi = ((u64::from(n) * u64::from(m)) >> 32) as u32;

        if add {
            assert!((1..=32).contains(&s));
            (hi.wrapping_add(n.wrapping_sub(hi) >> 1)) >> (s - 1)
        } else {
            assert!(s < 32);
            hi >> s
        }
    }

    fn magic_sdiv(n: i32, magic: (i32, u32)) -> i32 {
        let (m, s) = magic;
        assert!(s < 32);

        let hi = ((i64::from(n) * i64::from(m)) >> 32) as i32;
        let adjusted = if m < 0 { hi.wrapping_add(n) } else { hi };

        (adjusted >> s).wrapping_sub(n >> 31)
    }

    // Deterministic, dependency-free sampling. This is not a cryptographic
    // generator and is not used as a correctness oracle.
    fn next_u32(state: &mut u32) -> u32 {
        let mut x = *state;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        *state = x;
        x
    }

    fn check_unsigned(n: u32, d: u32, magic: (u32, u32, bool)) {
        assert_eq!(
            magic_udiv(n, magic),
            n / d,
            "unsigned n={n}, d={d}, magic={magic:?}"
        );
    }

    fn check_signed(n: i32, d: i32, magic: (i32, u32)) {
        let q = magic_sdiv(n, magic);

        assert_eq!(q, n / d, "signed n={n}, d={d}, magic={magic:?}");

        // d >= 2, so -d is representable and neither reference division
        // can encounter the i32::MIN / -1 overflow case.
        assert_eq!(
            q.wrapping_neg(),
            n / -d,
            "signed n={n}, negative divisor={}, magic={magic:?}",
            -d
        );
    }

    fn check_unsigned_divisor(d: u32) {
        assert!(d >= 2 && !d.is_power_of_two());
        let magic = magic_u32(d);

        for n in [
            0,
            1,
            2,
            d - 1,
            d,
            d.saturating_add(1),
            0x7fff_ffff,
            0x8000_0000,
            0x8000_0001,
            u32::MAX - 1,
            u32::MAX,
        ] {
            check_unsigned(n, d, magic);
        }

        // Probe immediately below, at, and above quotient transitions.
        // Arithmetic is widened before constructing the test inputs.
        for k in [0u32, 1, 2, 3, 7, 255, 65535, u32::MAX / d] {
            let multiple = u64::from(k) * u64::from(d);
            for n in [multiple.saturating_sub(1), multiple, multiple + 1] {
                if n <= u64::from(u32::MAX) {
                    check_unsigned(n as u32, d, magic);
                }
            }
        }

        let mut rng = d.wrapping_mul(0x9e37_79b9) | 1;
        for _ in 0..32 {
            check_unsigned(next_u32(&mut rng), d, magic);
        }
    }

    fn check_signed_divisor(d: i32) {
        assert!(d >= 2);
        let magic = magic_s32(d);

        for n in [
            i32::MIN,
            i32::MIN + 1,
            -d,
            -d + 1,
            -1,
            0,
            1,
            d - 1,
            d,
            d.saturating_add(1),
            i32::MAX - 1,
            i32::MAX,
        ] {
            check_signed(n, d, magic);
        }

        for k in [0i32, 1, 2, 3, 7, 255, 65535, i32::MAX / d, i32::MIN / d] {
            for sign in [-1i64, 1] {
                let multiple = sign * i64::from(k) * i64::from(d);
                for n in [multiple - 1, multiple, multiple + 1] {
                    if n >= i64::from(i32::MIN) && n <= i64::from(i32::MAX) {
                        check_signed(n as i32, d, magic);
                    }
                }
            }
        }

        let mut rng = (d as u32).wrapping_mul(0x85eb_ca6b) | 1;
        for _ in 0..32 {
            check_signed(next_u32(&mut rng) as i32, d, magic);
        }
    }

    #[test]
    fn unsigned_magic_matches_gcc_constants() {
        // Regression pins for the immediates GNU/LLVM/ICX emit (-m32 -O2).
        assert_eq!(magic_u32(3), (0xAAAA_AAABu32, 1, false));
        assert_eq!(magic_u32(5), (0xCCCC_CCCDu32, 2, false));
        assert_eq!(magic_u32(7), (0x2492_4925u32, 3, true));
        assert_eq!(magic_u32(11), (0xBA2E_8BA3u32, 3, false));
        assert_eq!(magic_u32(100), (0x51EB_851Fu32, 5, false));
    }

    #[test]
    fn signed_magic_matches_gcc_constants() {
        assert_eq!(magic_s32(3), (0x5555_5556i32, 0));
        assert_eq!(magic_s32(5), (0x6666_6667i32, 1));
        assert_eq!(magic_s32(7), (0x9249_2493u32 as i32, 2));
    }

    #[test]
    fn unsigned_known_parameters() {
        for (d, expected) in [
            (3, (0xaaaa_aaab, 1, false)),
            (5, (0xcccc_cccd, 2, false)),
            (7, (0x2492_4925, 3, true)),
            (11, (0xba2e_8ba3, 3, false)),
            (100, (0x51eb_851f, 5, false)),
        ] {
            assert_eq!(magic_u32(d), expected, "d={d}");
            check_unsigned_divisor(d);
        }
    }

    #[test]
    fn signed_known_parameters() {
        for (d, expected) in [
            (3, (0x5555_5556i32, 0)),
            (5, (0x6666_6667i32, 1)),
            (7, (0x9249_2493u32 as i32, 2)),
        ] {
            assert_eq!(magic_s32(d), expected, "d={d}");
            check_signed_divisor(d);
        }
    }

    #[test]
    fn unsigned_exhaustive_small_rectangle() {
        for d in 2u32..=255 {
            if d.is_power_of_two() {
                continue;
            }

            let magic = magic_u32(d);
            for n in 0u32..=4095 {
                check_unsigned(n, d, magic);
            }
        }
    }

    #[test]
    fn signed_exhaustive_small_rectangle() {
        // Unlike magic_u32, magic_s32 accepts positive powers of two.
        for d in 2i32..=255 {
            let magic = magic_s32(d);
            for n in -2048i32..=2047 {
                check_signed(n, d, magic);
            }
        }
    }

    #[test]
    fn unsigned_small_divisors_with_full_width_numerators() {
        for d in 2u32..=4096 {
            if !d.is_power_of_two() {
                check_unsigned_divisor(d);
            }
        }
    }

    #[test]
    fn signed_small_divisors_with_full_width_numerators() {
        for d in 2i32..=4096 {
            check_signed_divisor(d);
        }
    }

    #[test]
    fn unsigned_divisors_near_powers_of_two_and_maximum() {
        for bit in 1u32..=31 {
            let center = 1i64 << bit;
            for offset in -3i64..=3 {
                let d = center + offset;
                if d >= 2 && d <= i64::from(u32::MAX) {
                    let d = d as u32;
                    if !d.is_power_of_two() {
                        check_unsigned_divisor(d);
                    }
                }
            }
        }

        for offset in 0u32..=1024 {
            let d = u32::MAX - offset;
            if !d.is_power_of_two() {
                check_unsigned_divisor(d);
            }
        }
    }

    #[test]
    fn signed_divisors_near_powers_of_two_and_maximum() {
        // bit == 31 probes the largest valid positive signed divisors.
        for bit in 1u32..=31 {
            let center = 1i64 << bit;
            for offset in -3i64..=3 {
                let d = center + offset;
                if d >= 2 && d <= i64::from(i32::MAX) {
                    check_signed_divisor(d as i32);
                }
            }
        }

        for offset in 0i32..=1024 {
            check_signed_divisor(i32::MAX - offset);
        }
    }

    #[test]
    fn deterministic_full_width_divisor_samples() {
        let mut rng = 0x6a09_e667u32;

        for _ in 0..4096 {
            let unsigned = next_u32(&mut rng);
            if unsigned >= 2 && !unsigned.is_power_of_two() {
                check_unsigned_divisor(unsigned);
            }

            let signed = (next_u32(&mut rng) & 0x7fff_ffff) as i32;
            if signed >= 2 {
                check_signed_divisor(signed);
            }
        }
    }

    #[test]
    #[should_panic(expected = "non-power-of-two divisor >= 2")]
    fn unsigned_zero_is_rejected() {
        let _ = magic_u32(0);
    }

    #[test]
    #[should_panic(expected = "non-power-of-two divisor >= 2")]
    fn unsigned_one_is_rejected() {
        let _ = magic_u32(1);
    }

    #[test]
    #[should_panic(expected = "non-power-of-two divisor >= 2")]
    fn unsigned_power_of_two_is_rejected() {
        let _ = magic_u32(2);
    }

    #[test]
    #[should_panic(expected = "non-power-of-two divisor >= 2")]
    fn unsigned_high_power_of_two_is_rejected() {
        let _ = magic_u32(1u32 << 31);
    }

    #[test]
    #[should_panic(expected = "positive divisor >= 2")]
    fn signed_zero_is_rejected() {
        let _ = magic_s32(0);
    }

    #[test]
    #[should_panic(expected = "positive divisor >= 2")]
    fn signed_one_is_rejected() {
        let _ = magic_s32(1);
    }

    #[test]
    #[should_panic(expected = "positive divisor >= 2")]
    fn signed_negative_is_rejected() {
        let _ = magic_s32(-7);
    }

    #[test]
    #[should_panic(expected = "positive divisor >= 2")]
    fn signed_minimum_is_rejected() {
        let _ = magic_s32(i32::MIN);
    }
}
