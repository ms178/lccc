//! Compile-time magic numbers for division by constants (Hacker's Delight /
//! Granlund–Montgomery), i686-only. The IR-level `div_by_const` pass cannot run
//! on i686 (it emits 64-bit mulhi sequences the backend truncates), so the
//! codegen folds constant division itself with 32-bit `mull`/`imull` mulhi.
//!
//! The sequences match GCC/Clang/ICX exactly (verified against the godbolt
//! oracle): the magic values below reproduce the immediates GCC 16.2 emits for
//! `-m32 -O2` (see the unit tests).

/// Unsigned division magic. Returns `(mul, shift, add)`:
/// - `!add`: `q = mulhu(n, mul) >> shift`
/// - `add`:  `q = ((n - mulhu(n, mul)) >> 1 + mulhu(n, mul)) >> (shift - 1)`
pub(super) fn magic_u32(d: u32) -> (u32, u32, bool) {
    debug_assert!(d >= 2 && !d.is_power_of_two());
    let d64 = d as u64;
    let all_ones = 0xFFFF_FFFFu64;
    let nc = all_ones - (all_ones - d64 + 1) % d64; // = -1 - (-d)%d in 32 bits
    let half = 0x8000_0000u64;

    let mut p: u32 = 31;
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
        if !(q1 < delta || (q1 == delta && r1 == 0)) {
            break;
        }
    }

    ((q2 + 1) as u32, p - 32, add)
}

/// Signed division magic for a POSITIVE divisor `d >= 2`. Returns `(mul, shift)`:
///   `q = mulhs(n, mul) [ + n if mul < 0 ] >> shift - sign(n)`
/// (the final `- (n >> 31)` correction is applied by the emitter). Negative
/// divisors are handled by the emitter: `n / -d == -(n / d)`, so it calls this
/// with `|d|` and negates the quotient.
pub(super) fn magic_s32(d: i32) -> (i32, u32) {
    debug_assert!(d >= 2);
    let ad = d as u64;
    let two31: u64 = 1u64 << 31;
    let t = two31;
    let anc = t - 1 - t % ad;

    let mut p: u32 = 31;
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
        if !(q1 < delta || (q1 == delta && r1 == 0)) {
            break;
        }
    }

    ((q2 + 1) as i32, p - 32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsigned_magic_matches_gcc_constants() {
        // Constants reverse-engineered from GCC 16.2 -m32 -O2 (godbolt oracle).
        // d=3:  mulhi(n,0xAAAAAAAB) >> 1
        assert_eq!(magic_u32(3), (0xAAAA_AAABu32, 1, false));
        // d=5:  mulhi(n,0xCCCCCCCD) >> 2
        assert_eq!(magic_u32(5), (0xCCCC_CCCDu32, 2, false));
        // d=7:  add form, mul 0x24924925, >> 2 after (n - q)>>1 fixup
        assert_eq!(magic_u32(7), (0x2492_4925u32, 3, true));
        // d=11: mulhi(n,0xBA2E8BA3) >> 3
        assert_eq!(magic_u32(11), (0xBA2E_8BA3u32, 3, false));
        // d=100: mulhi(n,0x51EB851F) >> 5
        assert_eq!(magic_u32(100), (0x51EB_851Fu32, 5, false));
    }

    #[test]
    fn signed_magic_matches_gcc_constants() {
        // d=3: mulhs(n,0x55555556), shift 0
        assert_eq!(magic_s32(3), (0x5555_5556i32, 0));
        // d=5: mulhs(n,0x66666667) >> 1
        assert_eq!(magic_s32(5), (0x6666_6667i32, 1));
        // d=7: mulhs(n, 0x92492493) with add correction, shift 2
        //      (0x92492493 = -1840700269)
        assert_eq!(magic_s32(7), ((0x9249_2493u32 as i32), 2));
    }

    #[test]
    fn unsigned_magic_divides_correctly() {
        for d in [3u32, 5, 6, 7, 9, 10, 11, 12, 17, 25, 99, 100, 1000, 65537] {
            let (m, s, add) = magic_u32(d);
            for n in [0u32, 1, 2, d - 1, d, d + 1, 0xFFFF_FFFE, 0xFFFF_FFFF] {
                let q = magic_udiv(n, m, s, add);
                assert_eq!(q, n / d, "udiv by {d} of {n}");
            }
        }
    }

    #[test]
    fn signed_magic_divides_correctly() {
        for d in [3i32, 5, -5, 6, -6, 7, -7, 9, 10, 17, 100, -100, 1000] {
            let ad = d.unsigned_abs() as i32;
            let (m, s) = magic_s32(ad);
            for n in [0i32, 1, -1, d, -d, d - 1, i32::MAX, i32::MIN + 1] {
                let q = magic_sdiv(n, m, s);
                let q = if d < 0 { q.wrapping_neg() } else { q };
                assert_eq!(q, n / d, "sdiv by {d} of {n}");
            }
        }
    }

    // Reference implementations (mulhi via i64 to match the emitted mull/imull).
    fn magic_udiv(n: u32, m: u32, s: u32, add: bool) -> u32 {
        let hi = ((n as u64 * m as u64) >> 32) as u32;
        if add {
            (((n.wrapping_sub(hi)) >> 1).wrapping_add(hi)) >> (s - 1)
        } else {
            hi >> s
        }
    }
    fn magic_sdiv(n: i32, m: i32, s: u32) -> i32 {
        let hi = ((n as i64 * m as i64) >> 32) as i32;
        let mut q = hi;
        if m < 0 {
            q = q.wrapping_add(n);
        }
        if s > 0 {
            q >>= s;
        }
        q.wrapping_sub(n >> 31)
    }
}
