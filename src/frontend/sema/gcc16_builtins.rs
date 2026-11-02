//! GCC 16.2 / Clang 23 type-generic bit, carry-chain, bit-reversal,
//! 128-bit byte-swap, signaling-NaN, and speculation-barrier builtins.
//!
//! Shared between frontend semantic analysis (`sema/analysis.rs`,
//! `sema/type_checker.rs`, `sema/const_eval.rs`) and IR lowering
//! (`ir/lowering/const_eval.rs`, `ir/lowering/expr_types.rs`,
//! `ir/lowering/expr_builtins_gcc16.rs`).

use crate::common::types::{CType, target_is_32bit, target_ptr_size};

/// GCC 14/15/16.2 type-generic bit builtins (`clzg`, `ctzg`, `popcountg`,
/// `parityg` take unsigned non-`bool` integers; `ffsg`, `clrsbg` take signed
/// non-`bool` integers).
pub(crate) const GCC16_GENERIC_BIT_BUILTINS: [&str; 6] = [
    "__builtin_clzg",
    "__builtin_ctzg",
    "__builtin_popcountg",
    "__builtin_parityg",
    "__builtin_ffsg",
    "__builtin_clrsbg",
];

/// GCC / Clang multi-precision unsigned carry/borrow builtins.
pub(crate) const GCC16_CARRY_BUILTINS: [&str; 10] = [
    "__builtin_addcb",
    "__builtin_addcs",
    "__builtin_addc",
    "__builtin_addcl",
    "__builtin_addcll",
    "__builtin_subcb",
    "__builtin_subcs",
    "__builtin_subc",
    "__builtin_subcl",
    "__builtin_subcll",
];

/// 128-bit byte-swap, bit-reversal, and fixed-width rotate builtins.
pub(crate) const GCC16_BSWAP_BITREV_BUILTINS: [&str; 13] = [
    "__builtin_bswap128",
    "__builtin_bitreverse8",
    "__builtin_bitreverse16",
    "__builtin_bitreverse32",
    "__builtin_bitreverse64",
    "__builtin_rotateleft8",
    "__builtin_rotateleft16",
    "__builtin_rotateleft32",
    "__builtin_rotateleft64",
    "__builtin_rotateright8",
    "__builtin_rotateright16",
    "__builtin_rotateright32",
    "__builtin_rotateright64",
];

/// Return `(is_left, bit_width)` for `__builtin_rotate{left,right}{8,16,32,64}`.
pub(crate) fn rotate_builtin_info(name: &str) -> Option<(bool, u32)> {
    match name {
        "__builtin_rotateleft8" => Some((true, 8)),
        "__builtin_rotateleft16" => Some((true, 16)),
        "__builtin_rotateleft32" => Some((true, 32)),
        "__builtin_rotateleft64" => Some((true, 64)),
        "__builtin_rotateright8" => Some((false, 8)),
        "__builtin_rotateright16" => Some((false, 16)),
        "__builtin_rotateright32" => Some((false, 32)),
        "__builtin_rotateright64" => Some((false, 64)),
        _ => None,
    }
}

#[inline]
pub(crate) fn is_gcc16_generic_bit_builtin(name: &str) -> bool {
    GCC16_GENERIC_BIT_BUILTINS.contains(&name)
}

#[inline]
pub(crate) fn is_gcc16_carry_builtin(name: &str) -> bool {
    GCC16_CARRY_BUILTINS.contains(&name)
}

#[inline]
pub(crate) fn is_gcc16_bswap_bitrev_builtin(name: &str) -> bool {
    GCC16_BSWAP_BITREV_BUILTINS.contains(&name)
}

#[inline]
pub(crate) fn is_gcc16_extra_builtin(name: &str) -> bool {
    is_gcc16_generic_bit_builtin(name)
        || is_gcc16_carry_builtin(name)
        || is_gcc16_bswap_bitrev_builtin(name)
        || matches!(
            name,
            "__builtin_issignaling" | "__builtin_iseqsig" | "__builtin_speculation_safe_value"
        )
}

#[inline]
pub(crate) fn generic_bit_requires_signed(name: &str) -> bool {
    matches!(name, "__builtin_ffsg" | "__builtin_clrsbg")
}

#[inline]
pub(crate) fn generic_bit_allows_fallback(name: &str) -> bool {
    matches!(name, "__builtin_clzg" | "__builtin_ctzg")
}

/// Exact bit-width of an integer `CType` accepted by GCC 16 generic bit builtins.
pub(crate) fn int_bit_width_of_ctype(ct: &CType) -> Option<u32> {
    let ptr_bits = (target_ptr_size() * 8) as u32;
    match ct {
        CType::Char | CType::SChar | CType::UChar | CType::PlainUChar => Some(8),
        CType::Short | CType::UShort => Some(16),
        CType::Int | CType::UInt => Some(32),
        CType::Long | CType::ULong => Some(ptr_bits),
        CType::LongLong | CType::ULongLong => Some(64),
        CType::Int128 | CType::UInt128 => Some(128),
        CType::Enum(e) => int_bit_width_of_ctype(&e.compatible_ctype()),
        _ => None,
    }
}

/// Validate Argument 1 of a GCC 16.2 generic bit builtin, returning a
/// GCC-compatible error message on failure.
pub(crate) fn validate_generic_bit_arg0(name: &str, ct: &CType) -> Result<u32, String> {
    if matches!(ct, CType::Bool) {
        return Err(format!(
            "argument 1 in call to function '{name}' has boolean type"
        ));
    }
    if !ct.is_integer() {
        return Err(format!(
            "argument 1 in call to function '{name}' does not have integral type"
        ));
    }
    if generic_bit_requires_signed(name) {
        if !ct.is_signed() {
            return Err(format!(
                "argument 1 in call to function '{name}' has unsigned type"
            ));
        }
    } else if !ct.is_unsigned() {
        return Err(format!(
            "argument 1 in call to function '{name}' has signed type"
        ));
    }
    int_bit_width_of_ctype(ct).ok_or_else(|| {
        format!("argument 1 in call to function '{name}' does not have integral type")
    })
}

/// Validate optional Argument 2 of `__builtin_clzg` / `__builtin_ctzg`.
/// GCC 16.2 applies integer promotions and requires the promoted type to be `int`.
pub(crate) fn validate_generic_bit_fallback_arg(name: &str, ct: &CType) -> Result<(), String> {
    if !ct.is_integer() {
        return Err(format!(
            "argument 2 in call to function '{name}' does not have integral type"
        ));
    }
    if ct.integer_promoted() != CType::Int {
        return Err(format!(
            "argument 2 in call to function '{name}' does not have 'int' type"
        ));
    }
    Ok(())
}

/// Carry-chain builtin element `CType` and `(is_sub, bit_width)`.
pub(crate) fn carry_builtin_info(name: &str) -> Option<(CType, bool, u32)> {
    let ptr_bits = (target_ptr_size() * 8) as u32;
    match name {
        "__builtin_addcb" => Some((CType::UChar, false, 8)),
        "__builtin_addcs" => Some((CType::UShort, false, 16)),
        "__builtin_addc" => Some((CType::UInt, false, 32)),
        "__builtin_addcl" => Some((CType::ULong, false, ptr_bits)),
        "__builtin_addcll" => Some((CType::ULongLong, false, 64)),
        "__builtin_subcb" => Some((CType::UChar, true, 8)),
        "__builtin_subcs" => Some((CType::UShort, true, 16)),
        "__builtin_subc" => Some((CType::UInt, true, 32)),
        "__builtin_subcl" => Some((CType::ULong, true, ptr_bits)),
        "__builtin_subcll" => Some((CType::ULongLong, true, 64)),
        _ => None,
    }
}

/// Return `CType` of any builtin handled by this module.
pub(crate) fn gcc16_builtin_return_ctype(name: &str, arg0_ct: Option<&CType>) -> Option<CType> {
    if is_gcc16_generic_bit_builtin(name) {
        return Some(CType::Int);
    }
    if let Some((ty, _, _)) = carry_builtin_info(name) {
        return Some(ty);
    }
    match name {
        "__builtin_bswap128" => Some(CType::UInt128),
        "__builtin_bitreverse8" | "__builtin_rotateleft8" | "__builtin_rotateright8" => {
            Some(CType::UChar)
        }
        "__builtin_bitreverse16" | "__builtin_rotateleft16" | "__builtin_rotateright16" => {
            Some(CType::UShort)
        }
        "__builtin_bitreverse32" | "__builtin_rotateleft32" | "__builtin_rotateright32" => {
            Some(CType::UInt)
        }
        "__builtin_bitreverse64" | "__builtin_rotateleft64" | "__builtin_rotateright64" => {
            Some(if target_is_32bit() {
                CType::ULongLong
            } else {
                CType::ULong
            })
        }
        "__builtin_issignaling" | "__builtin_iseqsig" => Some(CType::Int),
        "__builtin_speculation_safe_value" => arg0_ct.cloned().or(Some(CType::Int)),
        _ => None,
    }
}

/// Evaluate GCC 16.2 generic bit builtins at compile time (`1..=128` bits).
pub(crate) fn eval_generic_bit_builtin(
    name: &str,
    raw_val: u128,
    fallback: Option<i64>,
    width: u32,
) -> Option<i64> {
    if width == 0 || width > 128 {
        return None;
    }
    let mask = if width == 128 {
        u128::MAX
    } else {
        (1u128 << width) - 1
    };
    let val = raw_val & mask;
    let leading_zeros = |v: u128| -> u32 {
        if v == 0 {
            width
        } else {
            let bit_w = 128 - v.leading_zeros();
            width.saturating_sub(bit_w)
        }
    };
    let trailing_zeros = |v: u128| -> u32 { if v == 0 { width } else { v.trailing_zeros() } };

    match name {
        "__builtin_clzg" => {
            if val == 0 {
                Some(fallback.unwrap_or(width as i64))
            } else {
                Some(leading_zeros(val) as i64)
            }
        }
        "__builtin_ctzg" => {
            if val == 0 {
                Some(fallback.unwrap_or(width as i64))
            } else {
                Some(trailing_zeros(val) as i64)
            }
        }
        "__builtin_popcountg" => Some(val.count_ones() as i64),
        "__builtin_parityg" => Some((val.count_ones() & 1) as i64),
        "__builtin_ffsg" => {
            if val == 0 {
                Some(0)
            } else {
                Some((trailing_zeros(val) + 1) as i64)
            }
        }
        "__builtin_clrsbg" => {
            let sign_bit = (val >> (width - 1)) & 1;
            let flipped = if sign_bit != 0 { (!val) & mask } else { val };
            if flipped == 0 {
                Some((width - 1) as i64)
            } else {
                let lz = leading_zeros(flipped);
                Some(lz.saturating_sub(1) as i64)
            }
        }
        _ => None,
    }
}

/// Evaluate `bswap128` and `bitreverse{8,16,32,64}` at compile time.
pub(crate) fn eval_bswap_or_bitrev(name: &str, raw_val: u128) -> Option<u128> {
    match name {
        "__builtin_bswap128" => Some(raw_val.swap_bytes()),
        "__builtin_bitreverse8" => Some((raw_val as u8).reverse_bits() as u128),
        "__builtin_bitreverse16" => Some((raw_val as u16).reverse_bits() as u128),
        "__builtin_bitreverse32" => Some((raw_val as u32).reverse_bits() as u128),
        "__builtin_bitreverse64" => Some((raw_val as u64).reverse_bits() as u128),
        _ => None,
    }
}

/// Evaluate `__builtin_rotate{left,right}{8,16,32,64}` at compile time.
pub(crate) fn eval_fixed_rotate(name: &str, raw_val: u64, raw_amt: u64) -> Option<u64> {
    let (is_left, bits) = rotate_builtin_info(name)?;
    let amt = (raw_amt as u32) & (bits - 1);
    Some(match (is_left, bits) {
        (true, 8) => (raw_val as u8).rotate_left(amt) as u64,
        (false, 8) => (raw_val as u8).rotate_right(amt) as u64,
        (true, 16) => (raw_val as u16).rotate_left(amt) as u64,
        (false, 16) => (raw_val as u16).rotate_right(amt) as u64,
        (true, 32) => (raw_val as u32).rotate_left(amt) as u64,
        (false, 32) => (raw_val as u32).rotate_right(amt) as u64,
        (true, 64) => raw_val.rotate_left(amt),
        (false, 64) => raw_val.rotate_right(amt),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gcc16_generic_bit_eval_matches_oracle() {
        assert_eq!(
            eval_generic_bit_builtin("__builtin_clzg", 8, None, 8),
            Some(4)
        );
        assert_eq!(
            eval_generic_bit_builtin("__builtin_clzg", 0, Some(99), 16),
            Some(99)
        );
        assert_eq!(
            eval_generic_bit_builtin("__builtin_clzg", 0, None, 32),
            Some(32)
        );
        assert_eq!(
            eval_generic_bit_builtin("__builtin_ctzg", 0, Some(-7), 16),
            Some(-7)
        );
        assert_eq!(
            eval_generic_bit_builtin("__builtin_popcountg", 0xFF, None, 8),
            Some(8)
        );
        assert_eq!(
            eval_generic_bit_builtin("__builtin_parityg", 0x7, None, 16),
            Some(1)
        );
        assert_eq!(
            eval_generic_bit_builtin("__builtin_ffsg", 0, None, 8),
            Some(0)
        );
        assert_eq!(
            eval_generic_bit_builtin("__builtin_ffsg", 4, None, 8),
            Some(3)
        );
        assert_eq!(
            eval_generic_bit_builtin("__builtin_clrsbg", 0, None, 8),
            Some(7)
        );
        assert_eq!(
            eval_generic_bit_builtin("__builtin_clrsbg", 0xFF, None, 8),
            Some(7)
        );
        assert_eq!(
            eval_generic_bit_builtin("__builtin_clrsbg", 0xFE, None, 8),
            Some(6)
        );
        assert_eq!(
            eval_generic_bit_builtin("__builtin_clrsbg", 0x7F, None, 8),
            Some(0)
        );
        assert_eq!(
            eval_generic_bit_builtin("__builtin_clrsbg", 0x80, None, 8),
            Some(0)
        );
    }

    #[test]
    fn bswap128_and_bitreverse_eval() {
        let x: u128 = 0x0102030405060708_1112131415161718;
        assert_eq!(
            eval_bswap_or_bitrev("__builtin_bswap128", x),
            Some(0x1817161514131211_0807060504030201)
        );
        assert_eq!(
            eval_bswap_or_bitrev("__builtin_bitreverse8", 0b1011_0001),
            Some(0b1000_1101)
        );
    }
}
