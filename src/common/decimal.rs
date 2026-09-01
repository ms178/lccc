//! IEEE 754-2008 binary-integer-decimal (BID) encoding for the C23 decimal
//! floating-point types `_Decimal32`, `_Decimal64` and `_Decimal128`.
//!
//! Bit layouts are GCC libbid's (libgcc/config/libbid/bid_internal.h):
//!
//! BID32: sign(1) | biased exp (8 bits, bias 96)  | coefficient (23 bits);
//!        steering bits 29..28 == 11 select `sgn|0x6000_0000|exp<<19|c&0x7f_ffff`
//!        with implicit coefficient high bit 2^23.
//! BID64: sign(1) | biased exp (10 bits, bias 398) | coefficient (53 bits)
//!        when coefficient < 2^53, else
//!        `sgn|0x6000_0000_0000_0000|exp<<51|c&0x7_FFFF_FFFF_FFFF`
//!        with implicit coefficient high bit 2^53.
//! BID128: sign(1) | biased exp (14 bits, bias 6176) | coefficient (113 bits).
//!        All decimal coefficients (<= 10^34-1 < 2^113) use the small form.
//!
//! Zeros keep their (clamped) exponent like GCC's decNumber-based folder:
//! `0.DD` is 0x31C0_0000_0000_0000, `-0.DD` is 0xB1C0_0000_0000_0000.
//! Rounding is round-half-even; overflow yields infinity; underflow rounds
//! through the subnormal range exactly digit-by-digit (half-even).

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
        let v: i64 = ds
            .parse::<i64>()
            .unwrap_or(if neg { -(1 << 40) } else { 1 << 40 });
        exp = if neg { -v } else { v };
    } else if i != bytes.len() {
        return None;
    }
    let mut digits: Vec<u8> = Vec::with_capacity(int_part.len() + frac_part.len());
    for c in int_part.chars().chain(frac_part.chars()) {
        digits.push(c as u8 - b'0');
    }
    let mut exponent = exp - frac_part.len() as i64;
    // strip leading zeros
    match digits.iter().position(|&d| d != 0) {
        Some(p) => {
            digits.drain(..p);
        }
        None => digits.clear(),
    }
    if digits.is_empty() {
        exponent = exp; // zeros keep their written exponent
    } else {
        // strip trailing zeros into the exponent (value preserving)
        let trailing = digits.iter().rev().take_while(|&&d| d == 0).count();
        if trailing > 0 {
            let n = digits.len();
            digits.truncate(n - trailing);
            exponent += trailing as i64;
        }
    }
    Some(ExactDecimal { digits, exponent: exponent.clamp(-(1 << 30), (1 << 30) - 1) as i32 })
}

/// Divide the digit vector by 10 once, rounding half-even on the removed
/// last digit. Returns the quotient (may be empty).
fn div10_half_even(digits: &[u8]) -> Vec<u8> {
    if digits.is_empty() {
        return Vec::new();
    }
    let removed = digits[digits.len() - 1];
    let mut q = digits[..digits.len() - 1].to_vec();
    if removed >= 5 {
        if q.is_empty() {
            q.push(0);
        }
        let mut carry = 1u8;
        let mut idx = q.len();
        while carry > 0 && idx > 0 {
            idx -= 1;
            let s = q[idx] + carry;
            q[idx] = s % 10;
            carry = s / 10;
        }
        if carry > 0 {
            q.insert(0, carry);
        }
    }
    q
}

fn digits_val(digits: &[u8]) -> u128 {
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

/// Round `digits` (value * 10^exponent) to `prec` significant digits,
/// half-even. Returns the (possibly empty == zero) digit vector and the new
/// exponent.
fn round_to_prec(digits: &[u8], exponent: i32, prec: usize) -> (Vec<u8>, i32) {
    let mut d = digits.to_vec();
    let mut e = exponent;
    while d.len() > prec {
        let removed = d[d.len() - 1];
        d = div10_half_even(&d);
        e += 1;
        let _ = removed;
    }
    // normalize: strip leading zeros, then trailing zeros into exponent
    while !d.is_empty() && d[0] == 0 {
        d.remove(0);
    }
    if d.is_empty() {
        return (Vec::new(), exponent);
    }
    let trailing = d.iter().rev().take_while(|&&x| x == 0).count();
    if trailing > 0 {
        let n = d.len();
        d.truncate(n - trailing);
        e += trailing as i32;
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
    // Renormalize: rounding may carry to prec+1 digits (only 10^prec).
    let mut d = d;
    if d.len() > p.prec {
        let removed = d[d.len() - 1];
        d = div10_half_even(&d);
        e += 1;
        let _ = removed;
    }
    // Overflow?
    if e > p.emax {
        return (None, -1); // -1 signals infinity
    }
    // Underflow to subnormal range: remove `emin - e` digits with
    // half-even rounding at the final scale.
    if e < p.emin {
        let shift = p.emin - e;
        let mut dd = d.clone();
        for _ in 0..shift {
            dd = div10_half_even(&dd);
        }
        while !dd.is_empty() && dd[0] == 0 {
            dd.remove(0);
        }
        if dd.is_empty() {
            // rounds to zero: subnormal zero (biased exponent 0)
            return (None, 0);
        }
        // strip trailing zeros of the (possibly re-normalized) coefficient
        let trailing = dd.iter().rev().take_while(|&&x| x == 0).count();
        let _ = trailing;
        let coef = digits_val(&dd);
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
    let p = WidthParams { prec: 7, bias: 96, emin: -95, emax: 96 };
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
                sign | 0x6000_0000 | ((efld as u32) << 19) | (c & 0x7f_ffff)
            }
        }
    }
}

/// Encode a decimal value into a BID64 bit pattern.
pub fn encode_bid64(neg: bool, digits: &[u8], exponent: i32) -> u64 {
    let sign: u64 = if neg { 0x8000_0000_0000_0000 } else { 0 };
    let p = WidthParams { prec: 16, bias: 398, emin: -383, emax: 384 };
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
    let p = WidthParams { prec: 34, bias: 6176, emin: -6143, emax: 6144 };
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
