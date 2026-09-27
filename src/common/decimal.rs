//! IEEE 754-2008 binary-integer-decimal (BID) encoding for the C23 decimal
//! floating-point types `_Decimal32`, `_Decimal64` and `_Decimal128`.
//!
//! Bit layouts are GCC libbid's (libgcc/config/libbid/bid_internal.h):
//!
//! BID32: sign(1) | biased exp (8 bits, bias 101) | coefficient (23 bits);
//!        top bits 11 (non-special) select large
//!        `sgn|0x6000_0000|exp<<21|c&0x1f_ffff` with implicit coefficient
//!        high bit 2^23; bit 28 is the exponent MSB, not steering (a
//!        biased exponent >= 128 sets it: 9000000e27DF is 0x70095440).
//! BID64: sign(1) | biased exp (10 bits, bias 398) | coefficient (53 bits)
//!        when coefficient < 2^53, else
//!        `sgn|0x6000_0000_0000_0000|exp<<51|c&0x7_FFFF_FFFF_FFFF`
//!        with implicit coefficient high bit 2^53; bit 60 is the
//!        exponent MSB (biased exponent >= 512 sets it:
//!        99000000000000000e114DD is 0x700B2BFF5F46C000).
//! BID128: sign(1) | biased exp (14 bits, bias 6176) | coefficient (113 bits).
//!        All decimal coefficients (<= 10^34-1 < 2^113) use the small form.
//!
//! Zeros keep their (clamped) exponent like GCC's decNumber-based folder:
//! `0.DD` is 0x31C0_0000_0000_0000, `-0.DD` is 0xB1C0_0000_0000_0000.
//! Rounding is round-half-even, decided ONCE against the full dropped
//! digit suffix (never digit-by-digit: sequential rounding with a sticky
//! bit double-rounds -- an intermediate round-up followed by an exact
//! half goes the wrong way, e.g. `1.000000451DF` must keep 1000000e-6,
//! the sequential version produced 1000001e-6); overflow yields infinity,
//! underflow rounds through the subnormal range by the same single
//! half-even decision.

use std::cmp::Ordering;

/// An exact decimal value: sign * digits * 10^exponent (digits most
/// significant first, no leading zeros; empty == zero).
#[derive(Debug, Clone)]
pub struct ExactDecimal {
    pub digits: Vec<u8>,
    pub exponent: i32,
}

impl ExactDecimal {
    pub fn is_zero(&self) -> bool {
        self.digits.is_empty() || self.digits.iter().all(|&d| d == 0)
    }
}

/// Parse a decimal floating literal body (digits, optional '.', optional
/// exponent, no sign/suffix) into an exact decimal value.
pub fn parse_decimal_literal(text: &str) -> Option<ExactDecimal> {
    let t = text.trim();
    let bytes = t.as_bytes();
    let mut i = 0usize;
    let mut int_part = String::new();
    let mut frac_part = String::new();
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        int_part.push(bytes[i] as char);
        i += 1;
    }
    if i < bytes.len() && bytes[i] == b'.' {
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            frac_part.push(bytes[i] as char);
            i += 1;
        }
    }
    if int_part.is_empty() && frac_part.is_empty() {
        return None;
    }
    let mut exp: i64 = 0;
    if i < bytes.len() && (bytes[i] == b'e' || bytes[i] == b'E') {
        i += 1;
        let mut neg = false;
        if i < bytes.len() && (bytes[i] == b'+' || bytes[i] == b'-') {
            neg = bytes[i] == b'-';
            i += 1;
        }
        let ds = &t[i..];
        if ds.is_empty() || !ds.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        // NB: `ds` holds the UNSIGNED magnitude (any sign was consumed
        // above), so the overflow sentinel must be positive here and the
        // sign applied exactly once below. (Negating a negative sentinel
        // would turn `1e-999...9` into +infinity instead of zero.)
        let mag: i64 = ds.parse::<i64>().unwrap_or(1 << 40);
        exp = if neg { -mag } else { mag };
    } else if i != bytes.len() {
        return None;
    }
    let mut digits: Vec<u8> = Vec::with_capacity(int_part.len() + frac_part.len());
    for c in int_part.chars().chain(frac_part.chars()) {
        digits.push(c as u8 - b'0');
    }
    let exponent = exp - frac_part.len() as i64;
    // strip leading zeros
    match digits.iter().position(|&d| d != 0) {
        Some(p) => {
            digits.drain(..p);
        }
        None => digits.clear(),
    }
    // No trailing-zero surgery: GCC preserves the written quantum
    // (1.50DF -> 150e-2, 10000000.DF -> 10000000e0, rounded later if the
    // width demands). Zeros keep their written exponent (`exp - frac_len`).
    Some(ExactDecimal {
        digits,
        exponent: exponent.clamp(-(1 << 30), (1 << 30) - 1) as i32,
    })
}

/// Divide the digit vector by 10 once, rounding half-even on the removed
/// last digit. `sticky` reports a nonzero digit below the removed one
/// (dropped by an earlier EXACT truncation, i.e. a genuine below-half
/// sticky from an unrounded suffix): an exact half (`removed == 5`, no
/// sticky) rounds to even, a half with sticky below sits above half and
/// rounds up. Returns the quotient (may be empty).
///
/// Single-step primitive: correct for ONE removal only. Do NOT chain
/// rounded steps through `sticky` -- sequential rounding double-rounds
/// (an intermediate round-up followed by an exact half goes the wrong
/// way; see `round_to_prec`). Production callers pass `sticky == false`
/// with an exact (zero) removed digit; the `sticky` arm exists to pin
/// the primitive's contract for exact-truncation callers.
fn div10_half_even(digits: &[u8], sticky: bool) -> Vec<u8> {
    if digits.is_empty() {
        return Vec::new();
    }
    let removed = digits[digits.len() - 1];
    let mut q = digits[..digits.len() - 1].to_vec();
    let kept_odd = q.last().is_some_and(|&d| d & 1 == 1);
    let round_up = removed > 5 || (removed == 5 && (sticky || kept_odd));
    if round_up {
        if q.is_empty() {
            q.push(0);
        }
        add1(&mut q);
    }
    q
}

fn digits_val(digits: &[u8]) -> u128 {
    // A 39-digit coefficient can exceed u128; all callers pass at most
    // `prec + 1` (<= 35) digits, so wrapping never triggers -- fail fast
    // in debug builds if that contract ever breaks.
    debug_assert!(
        digits.len() <= 38,
        "digits_val: coefficient exceeds u128 range"
    );
    let mut v: u128 = 0;
    for &d in digits {
        v = v.wrapping_mul(10).wrapping_add(d as u128);
    }
    v
}

fn add1(digits: &mut Vec<u8>) {
    let mut carry = 1u8;
    let mut idx = digits.len();
    while carry > 0 && idx > 0 {
        idx -= 1;
        let s = digits[idx] + carry;
        digits[idx] = s % 10;
        carry = s / 10;
    }
    if carry > 0 {
        digits.insert(0, carry);
    }
}

/// Compare a dropped digit suffix (nonempty, most significant first)
/// against half a kept unit (5 followed by zeros): `Greater` means the
/// suffix sits strictly above half (round up), `Equal` means it is an
/// exact half (half-even on the kept digit decides), `Less` means strictly
/// below half (round down).
fn cmp_suffix_to_half(suffix: &[u8]) -> Ordering {
    debug_assert!(!suffix.is_empty());
    match suffix[0].cmp(&5) {
        Ordering::Equal => {
            if suffix[1..].iter().any(|&x| x != 0) {
                Ordering::Greater
            } else {
                Ordering::Equal
            }
        }
        ord => ord,
    }
}

/// Round `digits` (value * 10^exponent) to `prec` significant digits,
/// half-even. Returns the (possibly empty == zero) digit vector and the new
/// exponent.
///
/// The decision is SINGLE: the entire dropped suffix is compared against
/// half a kept unit once (`cmp_suffix_to_half`). Rounding digit-by-digit
/// with a sticky bit -- even MPFR-style -- is WRONG here: an intermediate
/// round-up followed by an exact half double-rounds (the intermediate UP
/// corrupts the boundary digit while the stale sticky bit double-counts).
/// Witness: `1.000000451DF` is 1000000451e-9; sequential rounding yields
/// 1000001e-6, but the correctly rounded value (GCC-verified) is
/// 1000000e-6. This is also O(n) in the digit count; the old loop was
/// O(n^2) (a compile-time DoS on mega-literals).
fn round_to_prec(digits: &[u8], exponent: i32, prec: usize) -> (Vec<u8>, i32) {
    debug_assert!(prec > 0, "round_to_prec: precision must be nonzero");
    let mut d = digits.to_vec();
    let mut e = exponent;
    if d.len() > prec {
        let cut = d.len() - prec;
        let up = match cmp_suffix_to_half(&d[prec..]) {
            Ordering::Greater => true,
            Ordering::Less => false,
            Ordering::Equal => d[..prec].last().is_some_and(|&x| (x & 1) == 1),
        };
        d.truncate(prec);
        e = e.saturating_add(i32::try_from(cut).unwrap_or(i32::MAX));
        if up {
            add1(&mut d);
            // Carry growth (999 -> 1000) is at most one digit; shifting it
            // into the exponent is exact (the removed digit is 0, so no
            // rounding decision is involved).
            debug_assert!(d.len() <= prec + 1);
            if d.len() > prec {
                d = div10_half_even(&d, false);
                e = e.saturating_add(1);
            }
        }
    }
    // Normalize: strip leading zeros only (defensive; rounding a
    // normalized vector cannot create them, but div10 may yield [0] on an
    // exact-zero quotient). Trailing zeros are KEPT: GCC preserves the
    // rounded quantum verbatim (1.0000005DF -> 1000000e-6, int 10000000 ->
    // 1000000e1, float 0.1 -> 1000000e-7), so no digit surgery applies
    // after rounding.
    while !d.is_empty() && d[0] == 0 {
        d.remove(0);
    }
    if d.is_empty() {
        return (Vec::new(), exponent);
    }
    (d, e)
}

struct WidthParams {
    prec: usize,
    bias: i32,
    emin: i32,
    emax: i32,
}

/// Core encoder: returns the finite encoding fields (exp_field, coefficient
/// as u128) or None for zero. `sign_bit` handled by caller.
fn encode_fields(p: &WidthParams, digits: &[u8], exponent: i32) -> (Option<(i32, u128)>, i32) {
    let (d, mut e) = round_to_prec(digits, exponent, p.prec);
    if d.is_empty() {
        // zero: keep clamped written exponent
        let ef = (exponent + p.bias).clamp(0, p.emax + p.bias);
        return (None, ef);
    }
    // `round_to_prec` guarantees at most `prec` digits (a rounding carry
    // is shifted into the exponent there, exactly); assert the contract
    // rather than re-rounding here.
    debug_assert!(d.len() <= p.prec);
    let mut d = d;
    // Overflow rescue: an exponent past emax still fits when the
    // coefficient has headroom (1e91 = 10e90). Shift zeros into the
    // coefficient (value preserving) before declaring infinity.
    while d.len() < p.prec && e > p.emax {
        d.push(0);
        e -= 1;
    }
    // Overflow?
    if e > p.emax {
        return (None, -1); // -1 signals infinity
    }
    // Underflow to subnormal range: coef = round-half-even(D / 10^shift)
    // at biased exponent 0, where `shift = emin - e > 0`. Like
    // `round_to_prec` this is a SINGLE decision against the full dropped
    // suffix -- sequential div10 removal double-rounds (same witness
    // class: `1.000000451DF` near the boundary).
    if e < p.emin {
        let shift = p.emin - e;
        debug_assert!(shift > 0);
        // `d` is nonempty here (empty returned above) with a nonzero
        // leading digit (`round_to_prec` strips leading zeros).
        let n = d.len();
        debug_assert!(n > 0 && d[0] != 0);
        // Hang cap (F28): a hostile literal (`1e-999999999DF`) asks for
        // ~2^30 digit removals. Past `len` removals the quotient is
        // certainly subnormal zero (|D| / 10^shift < 0.1 rounds down):
        // return it in O(1) with bit-identical results.
        if shift as usize > n {
            return (None, 0);
        }
        if shift as usize == n {
            // Rounds to 0 or the minimum subnormal: the kept prefix is
            // empty (even), so an exact half rounds down to zero.
            let up = cmp_suffix_to_half(&d) == Ordering::Greater;
            if !up {
                return (None, 0);
            }
            return (Some((0, 1)), 0);
        }
        // shift < n: keep the leading `n - shift` digits, round once by
        // the `shift`-digit suffix.
        let keep = n - shift as usize;
        let mut k = d[..keep].to_vec();
        let up = match cmp_suffix_to_half(&d[keep..]) {
            Ordering::Greater => true,
            Ordering::Less => false,
            Ordering::Equal => k.last().is_some_and(|&x| (x & 1) == 1),
        };
        if up {
            add1(&mut k);
        }
        // `k` is nonempty with a nonzero leading digit (`d` is
        // normalized, `add1` preserves/grows the leading digit), so no
        // stripping is needed. The coefficient holds at most `prec`
        // digits... except a rounding carry (999 -> 1000) adds one; the
        // value stays exact either way.
        let coef = digits_val(&k);
        return (Some((0, coef)), 0);
    }
    let ef = e + p.bias;
    let coef = {
        let mut v: u128 = 0;
        for &dg in &d {
            v = v * 10 + dg as u128;
        }
        v
    };
    (Some((ef, coef)), ef)
}

/// Encode a decimal value into a BID32 bit pattern.
pub fn encode_bid32(neg: bool, digits: &[u8], exponent: i32) -> u32 {
    let sign: u32 = if neg { 0x8000_0000 } else { 0 };
    let p = WidthParams {
        prec: 7,
        // Decimal32: bias 101, subnormal floor -101; the exponent cap is
        // 90 = Emax-(prec-1) (IEEE Emax is 96, but GCC canonicalizes
        // larger exponents by filling the coefficient: 1e91 -> 10e90,
        // 1e96 -> 1000000e90, 1e97 -> inf; see the overflow rescue).
        bias: 101,
        emin: -101,
        emax: 90,
    };
    let (r, ef) = encode_fields(&p, digits, exponent);
    match r {
        None => {
            if ef < 0 {
                sign | 0x7800_0000 // infinity
            } else {
                sign | ((ef as u32) << 23) // zero (coefficient 0)
            }
        }
        Some((efld, coef)) => {
            let c = coef as u32;
            if c < (1 << 23) {
                sign | ((efld as u32) << 23) | c
            } else {
                sign | 0x6000_0000 | ((efld as u32) << 21) | (c & 0x1f_ffff)
            }
        }
    }
}

/// Encode a decimal value into a BID64 bit pattern.
pub fn encode_bid64(neg: bool, digits: &[u8], exponent: i32) -> u64 {
    let sign: u64 = if neg { 0x8000_0000_0000_0000 } else { 0 };
    let p = WidthParams {
        prec: 16,
        // Decimal64: bias 398, subnormal floor -398; the exponent cap is
        // 369 = Emax-(prec-1) (IEEE Emax is 384; GCC fills instead:
        // 1e370 -> 10e369, 1e384 -> 1e15e369, 1e385 -> inf).
        bias: 398,
        emin: -398,
        emax: 369,
    };
    let (r, ef) = encode_fields(&p, digits, exponent);
    match r {
        None => {
            if ef < 0 {
                sign | 0x7800_0000_0000_0000
            } else {
                sign | ((ef as u64) << 53)
            }
        }
        Some((efld, coef)) => {
            let c = coef as u64;
            if c < (1u64 << 53) {
                sign | ((efld as u64) << 53) | c
            } else {
                sign | 0x6000_0000_0000_0000 | ((efld as u64) << 51) | (c & 0x7_FFFF_FFFF_FFFF)
            }
        }
    }
}

/// Encode a decimal value into a BID128 bit pattern, returned (hi, lo).
pub fn encode_bid128(neg: bool, digits: &[u8], exponent: i32) -> (u64, u64) {
    let sign: u64 = if neg { 0x8000_0000_0000_0000 } else { 0 };
    let p = WidthParams {
        prec: 34,
        // Decimal128: bias 6176, subnormal floor -6176; the exponent cap
        // is 6111 = Emax-(prec-1) (IEEE Emax is 6144; GCC fills instead:
        // 1e6112 -> 10e6111, 1e6144 -> 1e33e6111, 1e6145 -> inf).
        bias: 6176,
        emin: -6176,
        emax: 6111,
    };
    let (r, ef) = encode_fields(&p, digits, exponent);
    match r {
        None => {
            if ef < 0 {
                (sign | 0x7800_0000_0000_0000, 0)
            } else {
                (sign | ((ef as u64) << 49), 0)
            }
        }
        Some((efld, coef)) => {
            debug_assert!(coef < (1u128 << 113), "D128 coefficients never reach 2^113");
            let hi = sign | ((efld as u64) << 49) | ((coef >> 64) as u64);
            let lo = coef as u64;
            (hi, lo)
        }
    }
}

/// Sign-flip helpers (BID sign is the top bit in every width).
pub fn negate_bid32(x: u32) -> u32 {
    x ^ 0x8000_0000
}
pub fn negate_bid64(x: u64) -> u64 {
    x ^ 0x8000_0000_0000_0000
}
pub fn negate_bid128_hi(hi: u64) -> u64 {
    hi ^ 0x8000_0000_0000_0000
}

/// Canonical BID infinities and quiet NaNs (GCC-compatible bit patterns;
/// measured against `gcc -S`: `_Decimal32 nan = NAN` is 0x7C000000).
pub const BID32_INF: u32 = 0x7800_0000;
pub const BID32_NAN: u32 = 0x7C00_0000;
pub const BID64_INF: u64 = 0x7800_0000_0000_0000;
pub const BID64_NAN: u64 = 0x7C00_0000_0000_0000;
pub const BID128_INF_HI: u64 = 0x7800_0000_0000_0000;
pub const BID128_NAN_HI: u64 = 0x7C00_0000_0000_0000;

// ---------------------------------------------------------------------------
// BID decoding + exact conversions (static-init support).
// ---------------------------------------------------------------------------

/// Decode finite BID32 bits into (negative, coefficient, base-10 exponent):
/// value = (-1)^neg * coeff * 10^exp. Returns None for infinities and NaNs
/// (combination field G[4:1] == 0b1111 in every width). Mirrors the decimal
/// battery's verified `dec32` python decoder, plus specials handling.
pub fn decode_bid32(v: u32) -> Option<(bool, u32, i32)> {
    if (v & 0x7800_0000) == 0x7800_0000 {
        return None;
    }
    let neg = (v & 0x8000_0000) != 0;
    // Large-coefficient form: top bits 11 (specials excluded above).
    // Bit 28 is the exponent MSB, NOT steering: a biased exponent >=
    // 128 (e >= 27) sets it, and requiring it clear misroutes large
    // values like 9000000e27DF (0x70095440) into the small arm (silent
    // wrong-code in cross-width conversions and casts of such constants).
    if (v & 0x6000_0000) == 0x6000_0000 {
        // Large-coefficient form: implicit high bit 2^23.
        let c = (1u32 << 23) | (v & 0x1F_FFFF);
        let e = (((v >> 21) & 0xFF) as i32) - 101;
        Some((neg, c, e))
    } else {
        let c = v & 0x7F_FFFF;
        let e = (((v >> 23) & 0xFF) as i32) - 101;
        Some((neg, c, e))
    }
}

/// Decode finite BID64 bits. See [`decode_bid32`].
pub fn decode_bid64(v: u64) -> Option<(bool, u64, i32)> {
    if (v & 0x7800_0000_0000_0000) == 0x7800_0000_0000_0000 {
        return None;
    }
    let neg = (v & 0x8000_0000_0000_0000) != 0;
    // Large-coefficient form: top bits 11 (specials excluded above).
    // Bit 60 is the exponent MSB, NOT steering: a biased exponent >=
    // 512 (e >= 114) sets it (see `decode_bid32` for the D32 sibling:
    // requiring it clear is silent wrong-code on large+bigexponent
    // constants in conversions and casts).
    if (v & 0x6000_0000_0000_0000) == 0x6000_0000_0000_0000 {
        // Large-coefficient form: implicit high bit 2^53.
        let c = (1u64 << 53) | (v & 0x7_FFFF_FFFF_FFFF);
        let e = (((v >> 51) & 0x3FF) as i32) - 398;
        Some((neg, c, e))
    } else {
        // Small form: full 53-bit coefficient (2^53 - 1 is FOURTEEN hex
        // digits; a 13-digit mask silently drops bits 52..49).
        let c = v & 0x1F_FFFF_FFFF_FFFF;
        let e = (((v >> 53) & 0x3FF) as i32) - 398;
        Some((neg, c, e))
    }
}

/// Decode finite BID128 bits (small form only: every decimal coefficient
/// up to 10^34 - 1 fits the 113-bit field). See [`decode_bid32`].
pub fn decode_bid128(hi: u64, lo: u64) -> Option<(bool, u128, i32)> {
    if (hi & 0x7800_0000_0000_0000) == 0x7800_0000_0000_0000 {
        return None;
    }
    let neg = (hi & 0x8000_0000_0000_0000) != 0;
    let c = (((hi & 0x1_FFFF_FFFF_FFFF) as u128) << 64) | lo as u128;
    let e = (((hi >> 49) & 0x3FFF) as i32) - 6176;
    Some((neg, c, e))
}

/// C-truthiness zero test on a BID32 carrier: +0 and -0 in any quantum
/// compare equal to zero; infinities and NaNs are nonzero. Powers the
/// constant evaluator's `is_zero` for `D32` (branch/logical folding).
pub fn bid32_is_zero(v: u32) -> bool {
    match decode_bid32(v) {
        Some((_, c, _)) => c == 0,
        None => false,
    }
}

/// C-truthiness zero test on a BID64 carrier. See [`bid32_is_zero`].
pub fn bid64_is_zero(v: u64) -> bool {
    match decode_bid64(v) {
        Some((_, c, _)) => c == 0,
        None => false,
    }
}

/// A u128 magnitude as most-significant-first decimal digits (`[0]` for
/// zero), matching the encoder's digit-vector convention.
fn int_digits(mag: u128) -> Vec<u8> {
    if mag == 0 {
        return vec![0];
    }
    let mut m = mag;
    let mut rev = Vec::new();
    while m > 0 {
        rev.push((m % 10) as u8);
        m /= 10;
    }
    rev.reverse();
    rev
}

/// Encode an integer magnitude into BID32, rounding half-even when the
/// magnitude exceeds 7 significant digits. Static initializers cannot
/// decline unrepresentable values the way the runtime folder does:
/// `_Decimal32 g = 123456789;` must round exactly like GCC.
pub fn encode_int_bid32(mag: u128, neg: bool) -> u32 {
    encode_bid32(neg, &int_digits(mag), 0)
}

/// Encode an integer magnitude into BID64 (16 digits). See
/// [`encode_int_bid32`].
pub fn encode_int_bid64(mag: u128, neg: bool) -> u64 {
    encode_bid64(neg, &int_digits(mag), 0)
}

/// Encode an integer magnitude into BID128, returned (hi, lo). See
/// [`encode_int_bid32`].
pub fn encode_int_bid128(mag: u128, neg: bool) -> (u64, u64) {
    encode_bid128(neg, &int_digits(mag), 0)
}

/// Multiply a most-significant-first digit vector by a small factor.
/// The accumulator is u64: 9 * 5^13 overflows u32 (caught by the exactness
/// test below, silently in optimized builds).
fn mul_small(digits: &mut Vec<u8>, m: u32) {
    debug_assert!(!digits.is_empty());
    let mut carry: u64 = 0;
    for d in digits.iter_mut().rev() {
        let t = *d as u64 * m as u64 + carry;
        *d = (t % 10) as u8;
        carry = t / 10;
    }
    let mut head = Vec::new();
    while carry > 0 {
        head.push((carry % 10) as u8);
        carry /= 10;
    }
    head.reverse();
    digits.splice(..0, head);
}

/// Exact decimal expansion of `mant * 2^exp2` (`mant > 0`) as
/// most-significant-first digits plus a base-10 exponent: value equals the
/// digit integer times 10^exp10, with no rounding whatsoever. Powers exact
/// binary-float -> BID static-init conversion (GCC/mpfr agree bit-for-bit;
/// shortest-repr printing would double-round on adversarial inputs).
/// Positive powers multiply by 2 (in 2^10 chunks); negative powers use
/// 2^-k = 5^k * 10^-k (exact, division-free, in 5^13 chunks).
pub fn binary_to_decimal_digits(mant: u128, exp2: i32) -> (Vec<u8>, i32) {
    debug_assert!(mant > 0);
    let mut digits = int_digits(mant);
    let mut exp10: i32 = 0;
    if exp2 > 0 {
        let mut k = exp2 as u32;
        while k >= 10 {
            mul_small(&mut digits, 1024);
            k -= 10;
        }
        if k > 0 {
            mul_small(&mut digits, 1u32 << k);
        }
    } else if exp2 < 0 {
        let mut k = (-(exp2 as i64)) as u32;
        while k >= 13 {
            mul_small(&mut digits, 1220703125); // 5^13
            k -= 13;
        }
        if k > 0 {
            mul_small(&mut digits, 5u32.pow(k));
        }
        exp10 = exp2;
    }
    // GCC binary->decimal quantum: exact digits strip trailing zeros,
    // then pad to a minimum of 2 digits (0.5f64 -> 50e-2, 7.0 -> 70e-1,
    // 100.0 -> 10e1; measured GCC 14 x86-64 at all widths incl. DL).
    // Literals and ints keep minimal quanta (no pad on those paths).
    while digits.len() > 1 && digits[digits.len() - 1] == 0 {
        digits.pop();
        exp10 += 1;
    }
    while digits.len() < 2 {
        digits.push(0);
        exp10 -= 1;
    }
    (digits, exp10)
}

/// A decoded binary floating-point value for exact decimal conversion.
pub enum BinaryFloat {
    Zero(bool),
    Inf(bool),
    Nan(bool),
    /// (negative, mantissa, power of two): value = mant * 2^exp2, mant > 0.
    Finite(bool, u128, i32),
}

/// Decode IEEE binary32 bits. Subnormals carry no hidden bit.
pub fn decode_f32_bits(b: u32) -> BinaryFloat {
    let neg = (b & 0x8000_0000) != 0;
    let e = ((b >> 23) & 0xFF) as i32;
    let m = b & 0x7F_FFFF;
    if e == 0xFF {
        if m == 0 {
            BinaryFloat::Inf(neg)
        } else {
            BinaryFloat::Nan(neg)
        }
    } else if e == 0 {
        if m == 0 {
            BinaryFloat::Zero(neg)
        } else {
            BinaryFloat::Finite(neg, m as u128, -149)
        }
    } else {
        BinaryFloat::Finite(neg, ((1u32 << 23) | m) as u128, e - 150)
    }
}

/// Decode IEEE binary64 bits. Subnormals carry no hidden bit.
pub fn decode_f64_bits(b: u64) -> BinaryFloat {
    let neg = (b & 0x8000_0000_0000_0000) != 0;
    let e = ((b >> 52) & 0x7FF) as i32;
    let m = b & 0xF_FFFF_FFFF_FFFF;
    if e == 0x7FF {
        if m == 0 {
            BinaryFloat::Inf(neg)
        } else {
            BinaryFloat::Nan(neg)
        }
    } else if e == 0 {
        if m == 0 {
            BinaryFloat::Zero(neg)
        } else {
            BinaryFloat::Finite(neg, m as u128, -1074)
        }
    } else {
        BinaryFloat::Finite(neg, ((1u64 << 52) | m) as u128, e - 1075)
    }
}

/// Decode IEEE binary128 bits (also the payload of `LongDouble`
/// constants, which pair an f64 approximation with binary128 bytes).
pub fn decode_f128_bits(b: u128) -> BinaryFloat {
    let neg = (b & (1u128 << 127)) != 0;
    let e = ((b >> 112) & 0x7FFF) as i32;
    let m = b & ((1u128 << 112) - 1);
    if e == 0x7FFF {
        if m == 0 {
            BinaryFloat::Inf(neg)
        } else {
            BinaryFloat::Nan(neg)
        }
    } else if e == 0 {
        if m == 0 {
            BinaryFloat::Zero(neg)
        } else {
            BinaryFloat::Finite(neg, m, -16494)
        }
    } else {
        BinaryFloat::Finite(neg, (1u128 << 112) | m, e - 16495)
    }
}

/// Classify BID32 specials as (negative, is_nan). Only meaningful when
/// [`decode_bid32`] declines: G == 0b11110 is infinity, 0b11111 is NaN.
pub fn bid32_special(v: u32) -> (bool, bool) {
    ((v & 0x8000_0000) != 0, (v & 0x0400_0000) != 0)
}

/// Classify BID64 specials. See [`bid32_special`].
pub fn bid64_special(v: u64) -> (bool, bool) {
    (
        (v & 0x8000_0000_0000_0000) != 0,
        (v & 0x0400_0000_0000_0000) != 0,
    )
}

/// Classify BID128 specials from the high word. See [`bid32_special`].
pub fn bid128_special_hi(hi: u64) -> (bool, bool) {
    (
        (hi & 0x8000_0000_0000_0000) != 0,
        (hi & 0x0400_0000_0000_0000) != 0,
    )
}

/// Fit a signed magnitude into a `bits`-wide integer (C truncation toward
/// zero is the caller's job; this only range-checks). Returns the i128 bit
/// pattern (two's complement for unsigned-wide values, matching the const
/// folder's `bits as i128` convention). None on overflow.
fn fit_int(mag: u128, neg: bool, bits: u32, unsigned: bool) -> Option<i128> {
    debug_assert!(matches!(bits, 8 | 16 | 32 | 64 | 128));
    if unsigned {
        if bits >= 128 {
            // u128 wrap is total: every magnitude lands in range.
            return Some(if neg {
                (0u128.wrapping_sub(mag)) as i128
            } else {
                mag as i128
            });
        }
        let max: u128 = (1u128 << bits) - 1;
        if neg {
            // C wraparound for near-range magnitudes (GCC folds
            // `(unsigned)-1.0` to 4294967295); far-out-of-range
            // float->unsigned is UB, so decline like GCC errors.
            if mag <= max + 1 {
                let wrapped = (max + 1 - (mag & max)) & max;
                Some(wrapped as i128)
            } else {
                None
            }
        } else if mag <= max {
            Some(mag as i128)
        } else {
            None
        }
    } else {
        let max_mag: u128 = if bits >= 128 {
            if neg {
                1u128 << 127
            } else {
                u128::MAX >> 1
            }
        } else if neg {
            1u128 << (bits - 1)
        } else {
            (1u128 << (bits - 1)) - 1
        };
        if mag > max_mag {
            return None;
        }
        if neg {
            // wrapping: mag == 2^127 negates to i128::MIN exactly.
            Some((mag as i128).wrapping_neg())
        } else {
            Some(mag as i128)
        }
    }
}

/// Truncate a BID value (sign * coeff * 10^exp10) to a `bits`-wide integer
/// (C semantics: toward zero). Powers constant `(int)decimal` casts. None
/// on overflow (GCC errors; we decline and the lenient path zeroes).
pub fn bid_to_int(
    coeff: u128,
    exp10: i32,
    neg: bool,
    bits: u32,
    unsigned: bool,
) -> Option<i128> {
    if coeff == 0 {
        return Some(0);
    }
    let mut mag = coeff;
    let mut e = exp10;
    while e > 0 {
        mag = mag.checked_mul(10)?;
        e -= 1;
    }
    while e < 0 {
        mag /= 10;
        e += 1;
        if mag == 0 {
            return Some(0);
        }
    }
    fit_int(mag, neg, bits, unsigned)
}

/// Truncate an IEEE binary128 value to a `bits`-wide integer (C semantics:
/// toward zero; exact mantissa shifting, never via f64). None for
/// NaN/Inf and overflow.
pub fn f128_to_int(bits: u128, target_bits: u32, target_unsigned: bool) -> Option<i128> {
    match decode_f128_bits(bits) {
        BinaryFloat::Zero(_) => Some(0),
        BinaryFloat::Inf(_) | BinaryFloat::Nan(_) => None,
        BinaryFloat::Finite(neg, mant, exp2) => {
            let mag = if exp2 >= 0 {
                mant.checked_shl(exp2 as u32)?
            } else {
                let s = (-(exp2 as i64)) as u32;
                if s >= 128 {
                    0
                } else {
                    mant >> s
                }
            };
            fit_int(mag, neg, target_bits, target_unsigned)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Split a u64 magnitude into most-significant-first decimal digits.
    fn digs(mut v: u64) -> Vec<u8> {
        if v == 0 {
            return vec![0];
        }
        let mut rev = Vec::new();
        while v > 0 {
            rev.push((v % 10) as u8);
            v /= 10;
        }
        rev.reverse();
        rev
    }

    /// F26: D32 integer encodings are bit-identical to GCC's `_Decimal32`
    /// rodata (small form bias 101, large form `E<<21|C21` — the old
    /// `<<19|C&0x7FFFFF` overlapped exponent and coefficient bits).
    #[test]
    fn bid32_int_vectors_match_gcc() {
        let enc = |v: i64| {
            let neg = v < 0;
            encode_bid32(neg, &digs(v.unsigned_abs()), 0)
        };
        assert_eq!(enc(0), 0x3280_0000);
        assert_eq!(enc(1), 0x3280_0001);
        assert_eq!(enc(5), 0x3280_0005);
        assert_eq!(enc(-5), 0xB280_0005);
        assert_eq!(enc(42), 0x3280_002A);
        assert_eq!(enc(127), 0x3280_007F);
        assert_eq!(enc(255), 0x3280_00FF);
        assert_eq!(enc(1_234_567), 0x3292_D687);
        assert_eq!(enc(8_388_607), 0x32FF_FFFF);
        assert_eq!(enc(8_388_608), 0x6CA0_0000);
        assert_eq!(enc(9_999_999), 0x6CB8_967F);
        assert_eq!(enc(-9_999_999), 0xECB8_967F);
        // Trailing-zero magnitudes ride the exponent (still exact).
        assert_eq!(encode_bid32(false, &[1], 7), 0x3600_0001); // 10^7
        assert_eq!(encode_bid32(false, &[1, 2, 3, 4, 5, 6, 7], 1), 0x3312_D687);
    }

    #[test]
    fn bid64_int_vectors_match_gcc() {
        let enc = |v: i64| {
            let neg = v < 0;
            encode_bid64(neg, &digs(v.unsigned_abs()), 0)
        };
        assert_eq!(enc(0), 0x31C0_0000_0000_0000);
        assert_eq!(enc(1), 0x31C0_0000_0000_0001);
        assert_eq!(enc(-1), 0xB1C0_0000_0000_0001);
        assert_eq!(enc(5), 0x31C0_0000_0000_0005);
        assert_eq!(enc(-42), 0xB1C0_0000_0000_002A);
        assert_eq!(enc(9_007_199_254_740_991), 0x31DF_FFFF_FFFF_FFFF);
        assert_eq!(enc(9_007_199_254_740_992), 0x6C70_0000_0000_0000);
        assert_eq!(enc(9_999_999_999_999_999), 0x6C73_86F2_6FC0_FFFF);
        assert_eq!(encode_bid64(false, &[1], 16), 0x33C0_0000_0000_0001);
        assert_eq!(encode_bid64(false, &[1], 18), 0x3400_0000_0000_0001);
    }

    #[test]
    fn bid32_fractional_values_match_gcc() {
        let enc = |digits: &[u8], exp: i32| encode_bid32(false, digits, exp);
        assert_eq!(enc(&[1, 5], -1), 0x3200_000F); // 1.5
        assert_eq!(enc(&[1], -1), 0x3200_0001); // 0.1
        assert_eq!(enc(&[1, 2, 3, 4, 5, 6], -3), 0x3101_E240); // 123.456
        // Canonical quantum 0; GCC emits cohort 0x32000032 (same value).
        assert_eq!(enc(&[5], 0), 0x3280_0005);
        // Zeros keep their written exponent, like GCC (`0.0DF`).
        assert_eq!(encode_bid32(false, &[], -1), 0x3200_0000);
        assert_eq!(encode_bid64(false, &[], -1), 0x31A0_0000_0000_0000);
    }

    /// F26: rounding is genuinely half-even (the old `removed >= 5`
    /// rounded every exact half up).
    #[test]
    fn rounding_is_half_even() {
        // Single-step halves: kept 6 (even) goes down, kept 7 (odd) up.
        assert_eq!(
            encode_bid32(false, &[1, 2, 3, 4, 5, 6, 6, 5], -8),
            0x2F12_D686
        );
        assert_eq!(
            encode_bid32(false, &[1, 2, 3, 4, 5, 6, 7, 5], -8),
            0x2F12_D688
        );
        // Multi-step (GCC preserves the rounded quantum verbatim, no
        // trailing strip): 1.0000005 -> 1000000e-6, 1.0000015 -> 1000002e-6.
        assert_eq!(
            encode_bid32(false, &[1, 0, 0, 0, 0, 0, 0, 5], -7),
            0x2F8F_4240
        );
        assert_eq!(
            encode_bid32(false, &[1, 0, 0, 0, 0, 0, 1, 5], -7),
            0x2F8F_4242
        );
        // Below half rounds down even with a nonzero tail: 1.00000015
        // -> 1000000e-6 (the dropped suffix [1,5] sits below half; only
        // an exact leading 5 consults the tail/kept parity).
        assert_eq!(
            encode_bid32(false, &[1, 0, 0, 0, 0, 0, 0, 1, 5], -8),
            0x2F8F_4240
        );
    }

    #[test]
    fn div10_half_even_pins() {
        assert_eq!(div10_half_even(&[], false), Vec::<u8>::new());
        assert_eq!(div10_half_even(&[4], false), Vec::<u8>::new());
        assert_eq!(div10_half_even(&[6], false), vec![1]);
        // Exact half: [5] -> [] (0 is even), [1,5] -> [2] (1 odd -> up),
        // [2,5] -> [2] (2 even -> down); sticky forces up.
        assert_eq!(div10_half_even(&[5], false), Vec::<u8>::new());
        assert_eq!(div10_half_even(&[5], true), vec![1]);
        assert_eq!(div10_half_even(&[1, 5], false), vec![2]);
        assert_eq!(div10_half_even(&[2, 5], false), vec![2]);
        assert_eq!(div10_half_even(&[2, 5], true), vec![3]);
    }

    #[test]
    fn overflow_yields_infinity() {
        // Coefficient headroom rescues e past the cap (GCC: 1e91DF is
        // finite 10e90, 1e370DD finite 10e369); only a full coefficient
        // past the cap is infinity. Caps: 90/369/6111 (Emax-(prec-1)).
        assert_eq!(encode_bid32(false, &[1], 90), 0x5F80_0001); // 1e90
        assert_eq!(encode_bid32(false, &[1], 91), 0x5F80_000A); // 1e91->10e90
        assert_eq!(encode_bid32(true, &[1], 91), 0xDF80_000A);
        assert_eq!(encode_bid32(false, &[1], 96), 0x5F8F_4240); // 1e96->1e6e90
        assert_eq!(encode_bid32(false, &[1], 97), 0x7800_0000); // +inf
        assert_eq!(encode_bid32(true, &[1], 97), 0xF800_0000); // -inf
        // Round-up past the max is also infinity (GCC: 9.9999999e96DF).
        assert_eq!(
            encode_bid32(false, &[9, 9, 9, 9, 9, 9, 9, 9], 89),
            0x7800_0000
        );
        assert_eq!(encode_bid64(false, &[1], 369), 0x5FE0_0000_0000_0001);
        assert_eq!(encode_bid64(false, &[1], 370), 0x5FE0_0000_0000_000A);
        assert_eq!(encode_bid64(false, &[1], 384), 0x5FE3_8D7E_A4C6_8000);
        assert_eq!(encode_bid64(false, &[1], 385), 0x7800_0000_0000_0000);
    }

    /// F26/F28: subnormal rounding at scale + the billion-shift hang cap
    /// (`1e-999999999DF` must return subnormal zero, not loop ~2^30 times).
    #[test]
    fn subnormal_and_hang_cap() {
        assert_eq!(encode_bid32(false, &[6], -102), 0x0000_0001);
        assert_eq!(encode_bid32(false, &[5], -102), 0x0000_0000);
        assert_eq!(encode_bid32(false, &[1], -999_999_999), 0x0000_0000);
        assert_eq!(encode_bid64(false, &[1], -999_999_999), 0x0000_0000_0000_0000);
    }

    #[test]
    fn parse_decimal_literal_pins() {
        let p = |t: &str| parse_decimal_literal(t).unwrap();
        assert_eq!(p("1.5").digits, vec![1, 5]);
        assert_eq!(p("1.5").exponent, -1);
        // Written quantum preserved (GCC: 1.50DF -> 150e-2).
        assert_eq!(p("5.0").digits, vec![5, 0]);
        assert_eq!(p("5.0").exponent, -1);
        assert!(p("0.0").digits.is_empty());
        assert_eq!(p("0.0").exponent, -1); // zeros keep written exponent
        assert_eq!(p("1e90").exponent, 90);
        assert_eq!(p("1E-5").exponent, -5);
        assert!(parse_decimal_literal("abc").is_none());
        assert!(parse_decimal_literal("").is_none());
        // The lexer strips DF/DD/DL before calling; a suffix here rejects.
        assert!(parse_decimal_literal("1.5DF").is_none());
    }

    #[test]
    fn bid128_smoke() {
        assert_eq!(
            encode_bid128(false, &[1, 5], -1),
            (0x303E_0000_0000_0000, 0xF)
        );
        assert_eq!(encode_bid128(false, &[], 0), (0x3040_0000_0000_0000, 0));
        // 1e6112 rescues to 10e6111; 1e6144 fills to 1e33e6111 (GCC);
        // a full coefficient past the cap is infinity.
        assert_eq!(
            encode_bid128(false, &[1], 6112),
            (0x5FFE_0000_0000_0000, 10)
        );
        assert_eq!(
            encode_bid128(false, &[1], 6144),
            (0x5FFE_314D_C644_8D93, 0x38C1_5B0A_0000_0000)
        );
        assert_eq!(
            encode_bid128(false, &[1], 6145),
            (0x7800_0000_0000_0000, 0)
        );
    }

    /// F31: the decoders invert the encoders on small forms, large forms
    /// (implicit high bit), zeros-with-exponent, and signed values; the
    /// special-field patterns (Inf/NaN) decline with None.
    #[test]
    fn decode_roundtrips_encoder() {
        assert_eq!(decode_bid32(0x3280_0005), Some((false, 5, 0)));
        assert_eq!(decode_bid32(0xB280_0005), Some((true, 5, 0)));
        assert_eq!(decode_bid32(0x3292_D687), Some((false, 1234567, 0)));
        // Large form: 2^23 implicit bit + 21 stored bits.
        assert_eq!(decode_bid32(0x6CA0_0000), Some((false, 8388608, 0)));
        assert_eq!(decode_bid32(0x6CB8_967F), Some((false, 9999999, 0)));
        // Trailing-zero quantum + zero with written exponent.
        assert_eq!(decode_bid32(0x3600_0001), Some((false, 1, 7)));
        assert_eq!(decode_bid32(0x3200_0000), Some((false, 0, -1)));
        assert_eq!(decode_bid32(0xB200_0000), Some((true, 0, -1)));
        // Specials decline.
        assert_eq!(decode_bid32(BID32_INF), None);
        assert_eq!(decode_bid32(BID32_NAN), None);
        assert_eq!(decode_bid32(BID32_INF | 0x8000_0000), None);
        assert_eq!(decode_bid32(BID32_NAN | 0x8000_0000), None);

        // 0x31C0... is zero with written exponent 0 ("0."/"0"); the
        // D32 0x32000000 sibling carries -1 ("0.0").
        assert_eq!(decode_bid64(0x31C0_0000_0000_0000), Some((false, 0, 0)));
        assert_eq!(
            decode_bid64(negate_bid64(0x31C0_0000_0000_0000)),
            Some((true, 0, 0))
        );
        // 2^53 takes the large form (implicit high bit 2^53).
        let big = encode_bid64(false, &digs(1 << 53), 0);
        assert_eq!(decode_bid64(big), Some((false, 1 << 53, 0)));
        assert_eq!(decode_bid64(BID64_INF), None);
        assert_eq!(decode_bid64(BID64_NAN), None);

        assert_eq!(
            decode_bid128(0x303E_0000_0000_0000, 0xF),
            Some((false, 15, -1))
        );
        assert_eq!(
            decode_bid128(BID128_INF_HI, 0x1234),
            None,
            "payload ignored on specials"
        );
        assert_eq!(decode_bid128(BID128_NAN_HI, 0), None);
    }

    /// F31: C truthiness on BID carriers — both zeros in any quantum are
    /// zero; specials and large-form values are nonzero.
    #[test]
    fn bid_is_zero_matches_c_truthiness() {
        assert!(bid32_is_zero(0x3280_0000)); // int 0
        assert!(bid32_is_zero(0x3200_0000)); // 0.0DF
        assert!(bid32_is_zero(0xB200_0000)); // -0.0DF
        assert!(!bid32_is_zero(0x3280_0001));
        assert!(!bid32_is_zero(0x6CA0_0000)); // large form: coeff 2^23
        assert!(!bid32_is_zero(BID32_INF));
        assert!(!bid32_is_zero(BID32_NAN));
        assert!(bid64_is_zero(0x31C0_0000_0000_0000));
        assert!(!bid64_is_zero(BID64_INF));
        assert!(!bid64_is_zero(BID64_NAN));
    }

    /// F31: integer magnitudes round half-even into BID widths (static
    /// initializers cannot decline like the runtime folder).
    #[test]
    fn encode_int_rounds_half_even() {
        // 123456789 -> 1234568e1 (dropped 89 > half).
        assert_eq!(
            decode_bid32(encode_int_bid32(123456789, false)),
            Some((false, 1234568, 2))
        );
        // Exact-half rounds to even: 1234565|5 -> 1234566e1.
        assert_eq!(
            decode_bid32(encode_int_bid32(12345655, false)),
            Some((false, 1234566, 1))
        );
        // u128 max into D32: 3402823|669... -> 3402824e32.
        assert_eq!(
            decode_bid32(encode_int_bid32(u128::MAX, true)),
            Some((true, 3402824, 32))
        );
        // 2^100 into D64: 1267650600228229|... first dropped 4 -> down.
        assert_eq!(
            decode_bid64(encode_int_bid64(1 << 100, false)),
            Some((false, 1267650600228229, 15))
        );
        // D128 takes 34 digits: 2^127-1 is exact (39 digits -> rounds).
        let (hi, lo) = encode_int_bid128((1 << 127) - 1, false);
        let (neg, c, e) = decode_bid128(hi, lo).unwrap();
        assert!(!neg);
        let s = c.to_string();
        assert_eq!(s.len(), 34, "rounds to 34 significant digits");
        assert!(s.starts_with("1701411834604692317316873037158841"));
        assert_eq!(e, 5);
    }

    /// F31: binary float bits decode to exact decimal expansions (ground
    /// truth: GCC/mpfr exact binary->decimal conversion).
    #[test]
    fn binary_to_decimal_is_exact() {
        // 1.5f64 is exact and short.
        let BinaryFloat::Finite(neg, m, e2) = decode_f64_bits(1.5f64.to_bits()) else {
            panic!("1.5 must be finite");
        };
        assert!(!neg);
        assert_eq!(binary_to_decimal_digits(m, e2), (vec![1, 5], -1));
        // 0.1f64: the full 55-digit exact expansion, not the shortest repr.
        let BinaryFloat::Finite(_, m, e2) = decode_f64_bits(0.1f64.to_bits()) else {
            panic!("0.1 must be finite");
        };
        let (d, e) = binary_to_decimal_digits(m, e2);
        let s: String = d.iter().map(|b| (b + b'0') as char).collect();
        assert_eq!(
            s,
            "1000000000000000055511151231257827021181583404541015625"
        );
        assert_eq!(e, -55);
        // f32 0.1: exact 0.100000001490116119384765625.
        let BinaryFloat::Finite(_, m, e2) = decode_f32_bits(0.1f32.to_bits()) else {
            panic!("f32 0.1 must be finite");
        };
        let (d, e) = binary_to_decimal_digits(m, e2);
        let s: String = d.iter().map(|b| (b + b'0') as char).collect();
        assert_eq!(s, "100000001490116119384765625");
        assert_eq!(e, -27);
        // Smallest denormal: 2^-1074 = 5^1074 * 10^-1074.
        let BinaryFloat::Finite(_, m, e2) = decode_f64_bits(1) else {
            panic!("denormal must be finite");
        };
        assert_eq!((m, e2), (1, -1074));
        let (d, e) = binary_to_decimal_digits(m, e2);
        assert_eq!(e, -1074);
        assert_eq!(d.len(), 751, "5^1074 has 751 digits");
        assert_eq!((d[0], d[750]), (4, 5));
        // 1e300: exact 300-digit integer starting 1000...0525....
        let BinaryFloat::Finite(_, m, e2) = decode_f64_bits(1e300f64.to_bits()) else {
            panic!("1e300 must be finite");
        };
        let (d, e) = binary_to_decimal_digits(m, e2);
        // The exact expansion ends in ...540160: one trailing zero lifts.
        assert_eq!(e, 1);
        assert_eq!(d[d.len() - 1], 6);
        let s: String = d.iter().take(32).map(|b| (b + b'0') as char).collect();
        assert_eq!(s, "10000000000000000525047602552044");
        // Specials and signed zeros.
        assert!(matches!(
            decode_f64_bits(f64::INFINITY.to_bits()),
            BinaryFloat::Inf(false)
        ));
        assert!(matches!(
            decode_f64_bits(f64::NAN.to_bits()),
            BinaryFloat::Nan(_)
        ));
        assert!(matches!(
            decode_f64_bits((-0.0f64).to_bits()),
            BinaryFloat::Zero(true)
        ));
    }

    /// F31: BID -> integer truncation (C toward-zero) with range checks.
    #[test]
    fn bid_to_int_truncates_and_range_checks() {
        // 25e-1 -> 2; -25e-1 -> -2 (toward zero, not floor).
        assert_eq!(bid_to_int(25, -1, false, 32, false), Some(2));
        assert_eq!(bid_to_int(25, -1, true, 32, false), Some(-2));
        assert_eq!(bid_to_int(9, -1, false, 32, false), Some(0));
        assert_eq!(bid_to_int(9, -1, true, 32, false), Some(0));
        // Positive exponents multiply.
        assert_eq!(bid_to_int(15, 1, false, 32, false), Some(150));
        // Boundaries: i32::MAX ok, +1 declines; i32::MIN ok.
        assert_eq!(bid_to_int(2147483647, 0, false, 32, false), Some(2147483647));
        assert_eq!(bid_to_int(2147483648, 0, false, 32, false), None);
        assert_eq!(bid_to_int(2147483648, 0, true, 32, false), Some(-2147483648));
        assert_eq!(bid_to_int(2147483649, 0, true, 32, false), None);
        // Unsigned wrap for near-range negatives (GCC folds these).
        assert_eq!(bid_to_int(1, 0, true, 32, true), Some(4294967295));
        assert_eq!(bid_to_int(256, 0, true, 8, true), Some(0));
        assert_eq!(bid_to_int(257, 0, true, 8, true), None);
        // Far out of range declines (UB; GCC errors).
        assert_eq!(bid_to_int(1, 300, false, 64, false), None);
        assert_eq!(bid_to_int(1, 300, false, 64, true), None);
    }

    /// F31: binary128 -> integer is exact (mantissa shifting, never f64).
    /// 2^60+1 is the f64-rounding trap: f64 sees 2^60, we see 2^60+1.
    #[test]
    fn f128_to_int_is_exact() {
        // 5.0F128 -> 5 (1.25 * 2^2: biased exp 16385, fraction 2^110).
        let five: u128 = (16385u128 << 112) | (1u128 << 110);
        assert_eq!(f128_to_int(five, 64, false), Some(5));
        assert_eq!(f128_to_int(five, 32, true), Some(5));
        // -7.5F128 -> -7 (toward zero, not floor).
        let neg75: u128 = (1u128 << 127) | (16385u128 << 112) | (7u128 << 109);
        assert_eq!(f128_to_int(neg75, 64, false), Some(-7));
        // 2^60 + 1 exactly (1.00..01 * 2^60: biased exp 16443, the +1 at
        // mantissa bit 52).
        let big: u128 = (16443u128 << 112) | (1u128 << 52);
        assert_eq!(f128_to_int(big, 64, false), Some((1i128 << 60) + 1));
        // 2^63 overflows i64, fits u64/i128.
        let p63: u128 = 16446u128 << 112;
        assert_eq!(f128_to_int(p63, 64, false), None);
        assert_eq!(f128_to_int(p63, 64, true), Some(1i128 << 63));
        assert_eq!(f128_to_int(p63, 128, false), Some(1i128 << 63));
        // Inf/NaN decline.
        assert_eq!(f128_to_int(0x7FFF_0000_0000_0000_0000_0000_0000_0000, 64, false), None);
        assert_eq!(f128_to_int(0x7FFF_8000_0000_0000_0000_0000_0000_0000, 64, false), None);
    }

    /// F31: special classifiers feed cross-width NaN/Inf propagation.
    #[test]
    fn special_classifiers() {
        assert_eq!(bid32_special(BID32_INF), (false, false));
        assert_eq!(bid32_special(BID32_NAN), (false, true));
        assert_eq!(bid32_special(BID32_NAN | 0x8000_0000), (true, true));
        assert_eq!(bid64_special(BID64_INF), (false, false));
        assert_eq!(bid64_special(BID64_NAN), (false, true));
        assert_eq!(bid128_special_hi(BID128_INF_HI), (false, false));
        assert_eq!(bid128_special_hi(BID128_NAN_HI), (false, true));
    }

    /// F33 (double-rounding regression): rounding is a SINGLE decision
    /// against the full dropped suffix. Sequential digit-by-digit
    /// rounding with a sticky bit double-rounds: an intermediate
    /// round-up followed by an exact half goes the wrong way. Witnesses
    /// (GCC-verified: `gcc -S` emits 0x2F8F4240 for both): 1.000000451DF
    /// and 1.0000004999DF both keep 1000000e-6; the sequential version
    /// produced 1000001e-6 for both.
    #[test]
    fn rounding_is_single_decision_no_double_round() {
        // 1.000000451: suffix [4,5,1] < half (first dropped digit 4) --
        // the 5 below the boundary must not leak upward.
        let lit = parse_decimal_literal("1.000000451").unwrap();
        assert_eq!(
            encode_bid32(false, &lit.digits, lit.exponent),
            0x2F8F_4240
        );
        // 1.0000004999: the 999-carry reaches the boundary 4 and makes
        // 5, but the full suffix [4,9,9,9] still sits below half.
        let lit = parse_decimal_literal("1.0000004999").unwrap();
        assert_eq!(
            encode_bid32(false, &lit.digits, lit.exponent),
            0x2F8F_4240
        );
        // Genuine above-half still rounds up: 1.000000501 -> 1000001e-6
        // (suffix [5,0,1] > half; GCC agrees: 0x2F8F4241).
        let lit = parse_decimal_literal("1.000000501").unwrap();
        assert_eq!(
            encode_bid32(false, &lit.digits, lit.exponent),
            0x2F8F_4241
        );
    }

    /// F33 (subnormal double-rounding regression): the subnormal path
    /// uses the same single decision. Witness (GCC-verified: `gcc -S`
    /// emits 2): 2.495e-101DF rounds to subnormal coefficient 2, not 3
    /// -- sequential div10 removal produced 3 (an intermediate round-up
    /// from the trailing 5, then a stale-sticky exact half going up).
    #[test]
    fn subnormal_rounding_is_single_decision() {
        let lit = parse_decimal_literal("2.495e-101").unwrap();
        assert_eq!(encode_bid32(false, &lit.digits, lit.exponent), 0x0000_0002);
    }

    /// F33: an exponent that overflows i64 keeps its sign -- the
    /// magnitude saturates, the sign applies once. `1e-999...9` is zero
    /// (not +infinity), `1e+999...9` is +infinity (GCC-verified values;
    /// GCC additionally warns -Woverflow, which is separate diagnostic
    /// work).
    #[test]
    fn parse_exponent_overflow_keeps_sign() {
        let lo = parse_decimal_literal("1e-99999999999999999999").unwrap();
        assert_eq!(lo.exponent, -(1 << 30));
        assert_eq!(encode_bid32(false, &lo.digits, lo.exponent), 0x0000_0000);
        let hi = parse_decimal_literal("1e+99999999999999999999").unwrap();
        assert_eq!(hi.exponent, (1 << 30) - 1);
        assert_eq!(encode_bid32(false, &hi.digits, hi.exponent), 0x7800_0000);
    }

    /// F33 (decoder steering regression): in the large-coefficient form
    /// the bit below the top pair is the exponent MSB, not steering --
    /// requiring it clear misroutes large+big-exponent values into the
    /// small arm (silent wrong-code in cross-width conversions and
    /// casts of such constants). Ground truth: `gcc -S` bit patterns.
    #[test]
    fn decode_large_with_exponent_msb() {
        // 9000000e27DF: large form, biased exponent 128 (bit 28 set).
        assert_eq!(
            decode_bid32(0x7009_5440),
            Some((false, 9000000, 27))
        );
        // Boundary: smallest large coefficient (2^23) with bit 28 set.
        assert_eq!(
            decode_bid32(0x7000_0000),
            Some((false, 8388608, 27))
        );
        // Encoder/decoder roundtrip at the boundary.
        assert_eq!(
            decode_bid32(encode_bid32(false, &[9, 0, 0, 0, 0, 0, 0], 27)),
            Some((false, 9000000, 27))
        );
        // D64 sibling: 99000000000000000e114DD rounds to
        // 9900000000000000e115, large form, biased exponent 513 (bit 60
        // set); GCC emits 0x700B2BFF5F46C000.
        assert_eq!(
            decode_bid64(0x700B_2BFF_5F46_C000),
            Some((false, 9900000000000000, 115))
        );
        assert_eq!(
            decode_bid64(encode_bid64(
                false,
                &[9, 9, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
                114
            )),
            Some((false, 9900000000000000, 115))
        );
    }
}
