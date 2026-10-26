//! C23 / C2y `<stdbit.h>` (`__builtin_stdc_*`) semantics and compile-time
//! constant evaluation.
//!
//! Shared between frontend semantic analysis (`sema/const_eval.rs`,
//! `sema/type_checker.rs`, `sema/analysis.rs`) and IR lowering
//! (`ir/lowering/const_eval.rs`, `ir/lowering/expr_builtins_intrin.rs`).

use crate::common::types::CType;

/// All 16 GCC 16 / C23 / C2y `__builtin_stdc_*` type-generic bit builtins.
pub(crate) const STDC_BIT_BUILTINS: [&str; 16] = [
    "__builtin_stdc_leading_zeros",
    "__builtin_stdc_leading_ones",
    "__builtin_stdc_trailing_zeros",
    "__builtin_stdc_trailing_ones",
    "__builtin_stdc_first_leading_zero",
    "__builtin_stdc_first_leading_one",
    "__builtin_stdc_first_trailing_zero",
    "__builtin_stdc_first_trailing_one",
    "__builtin_stdc_count_zeros",
    "__builtin_stdc_count_ones",
    "__builtin_stdc_has_single_bit",
    "__builtin_stdc_bit_width",
    "__builtin_stdc_bit_floor",
    "__builtin_stdc_bit_ceil",
    "__builtin_stdc_rotate_left",
    "__builtin_stdc_rotate_right",
];

#[inline]
pub(crate) fn is_stdc_bit_builtin(name: &str) -> bool {
    STDC_BIT_BUILTINS.contains(&name)
}

#[inline]
pub(crate) fn stdc_bit_arity(name: &str) -> Option<usize> {
    match name {
        "__builtin_stdc_rotate_left" | "__builtin_stdc_rotate_right" => Some(2),
        _ if is_stdc_bit_builtin(name) => Some(1),
        _ => None,
    }
}

/// Returns `true` when the builtin preserves the exact unpromoted unsigned
/// integer type of its first argument (`bit_floor`, `bit_ceil`, `rotate_left`,
/// `rotate_right`).
#[inline]
pub(crate) fn stdc_bit_returns_arg_type(name: &str) -> bool {
    matches!(
        name,
        "__builtin_stdc_bit_floor"
            | "__builtin_stdc_bit_ceil"
            | "__builtin_stdc_rotate_left"
            | "__builtin_stdc_rotate_right"
    )
}

#[inline]
pub(crate) fn stdc_bit_returns_bool(name: &str) -> bool {
    name == "__builtin_stdc_has_single_bit"
}

/// C23 7.18.1p2 / C2y 7.18.1p2: the generic `<stdbit.h>` functions accept
/// standard unsigned integer types, extended unsigned integer types
/// (`unsigned __int128`), and bit-precise unsigned integer types (`unsigned
/// _BitInt(N)`), excluding `bool` and plain `char`.
pub(crate) fn is_valid_stdc_bit_arg_ctype(ct: &CType) -> bool {
    matches!(
        ct,
        CType::UChar
            | CType::UShort
            | CType::UInt
            | CType::ULong
            | CType::ULongLong
            | CType::UInt128
    )
}

/// Bit width of a valid `<stdbit.h>` unsigned integer argument type.
pub(crate) fn stdc_bit_width_of_ctype(ct: &CType) -> Option<u32> {
    let ptr_bits = (crate::common::types::target_ptr_size() * 8) as u32;
    match ct {
        CType::UChar => Some(8),
        CType::UShort => Some(16),
        CType::UInt => Some(32),
        CType::ULong => Some(ptr_bits),
        CType::ULongLong => Some(64),
        CType::UInt128 => Some(128),
        _ => None,
    }
}

/// Return type of a `__builtin_stdc_*` call given the first argument's `CType`.
pub(crate) fn stdc_bit_return_ctype(name: &str, arg0_ct: Option<&CType>) -> Option<CType> {
    if !is_stdc_bit_builtin(name) {
        return None;
    }
    if stdc_bit_returns_bool(name) {
        return Some(CType::Bool);
    }
    if stdc_bit_returns_arg_type(name) {
        return arg0_ct.cloned().or(Some(CType::UInt));
    }
    Some(CType::UInt)
}

/// Evaluate any of the 16 `__builtin_stdc_*` builtins at compile time on an
/// unsigned value of exact bit-width `width` (`1..=128`).
pub(crate) fn eval_stdc_bit_builtin(
    name: &str,
    raw_val: u128,
    rotate_cnt: Option<i128>,
    width: u32,
) -> Option<u128> {
    if width == 0 || width > 128 {
        return None;
    }
    let mask = if width == 128 {
        u128::MAX
    } else {
        (1u128 << width) - 1
    };
    let x = raw_val & mask;
    let clz = |v: u128| -> u32 {
        if v == 0 {
            width
        } else {
            v.leading_zeros() - (128 - width)
        }
    };
    let ctz = |v: u128| -> u32 {
        if v == 0 {
            width
        } else {
            v.trailing_zeros().min(width)
        }
    };

    let res = match name {
        "__builtin_stdc_leading_zeros" => clz(x) as u128,
        "__builtin_stdc_leading_ones" => clz((!x) & mask) as u128,
        "__builtin_stdc_trailing_zeros" => ctz(x) as u128,
        "__builtin_stdc_trailing_ones" => ctz((!x) & mask) as u128,
        "__builtin_stdc_first_leading_zero" => {
            if x == mask {
                0
            } else {
                (clz((!x) & mask) + 1) as u128
            }
        }
        "__builtin_stdc_first_leading_one" => {
            if x == 0 {
                0
            } else {
                (clz(x) + 1) as u128
            }
        }
        "__builtin_stdc_first_trailing_zero" => {
            if x == mask {
                0
            } else {
                (ctz((!x) & mask) + 1) as u128
            }
        }
        "__builtin_stdc_first_trailing_one" => {
            if x == 0 {
                0
            } else {
                (ctz(x) + 1) as u128
            }
        }
        "__builtin_stdc_count_ones" => x.count_ones() as u128,
        "__builtin_stdc_count_zeros" => (width - x.count_ones()) as u128,
        "__builtin_stdc_has_single_bit" => u128::from(x != 0 && (x & (x - 1)) == 0),
        "__builtin_stdc_bit_width" => (width - clz(x)) as u128,
        "__builtin_stdc_bit_floor" => {
            if x == 0 {
                0
            } else {
                let bw = width - clz(x);
                1u128 << (bw - 1)
            }
        }
        "__builtin_stdc_bit_ceil" => {
            if x <= 1 {
                1 & mask
            } else {
                let bw = width - clz(x - 1);
                if bw >= width {
                    // C23 / GCC 16 wraps modulo 2^width to 0 when the next power
                    // of two is not representable in the return type.
                    0
                } else {
                    (1u128 << bw) & mask
                }
            }
        }
        "__builtin_stdc_rotate_left" => {
            let cnt = rotate_cnt?;
            let r = cnt.rem_euclid(width as i128) as u32;
            if r == 0 {
                x
            } else {
                ((x << r) | (x >> (width - r))) & mask
            }
        }
        "__builtin_stdc_rotate_right" => {
            let cnt = rotate_cnt?;
            let r = cnt.rem_euclid(width as i128) as u32;
            if r == 0 {
                x
            } else {
                ((x >> r) | (x << (width - r))) & mask
            }
        }
        _ => return None,
    };
    Some(res)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stdbit_8bit_and_32bit_edge_cases_match_c2y() {
        assert_eq!(
            eval_stdc_bit_builtin("__builtin_stdc_leading_zeros", 0, None, 8),
            Some(8)
        );
        assert_eq!(
            eval_stdc_bit_builtin("__builtin_stdc_leading_ones", 0xFF, None, 8),
            Some(8)
        );
        assert_eq!(
            eval_stdc_bit_builtin("__builtin_stdc_first_leading_zero", 0xFF, None, 8),
            Some(0)
        );
        assert_eq!(
            eval_stdc_bit_builtin("__builtin_stdc_first_leading_zero", 0xFE, None, 8),
            Some(8)
        );
        assert_eq!(
            eval_stdc_bit_builtin("__builtin_stdc_first_leading_one", 0, None, 8),
            Some(0)
        );
        assert_eq!(
            eval_stdc_bit_builtin("__builtin_stdc_first_leading_one", 1, None, 8),
            Some(8)
        );
        assert_eq!(
            eval_stdc_bit_builtin("__builtin_stdc_first_trailing_zero", 0xFF, None, 8),
            Some(0)
        );
        assert_eq!(
            eval_stdc_bit_builtin("__builtin_stdc_first_trailing_zero", 0x01, None, 8),
            Some(2)
        );
        assert_eq!(
            eval_stdc_bit_builtin("__builtin_stdc_count_zeros", 0xAA, None, 8),
            Some(4)
        );
        assert_eq!(
            eval_stdc_bit_builtin("__builtin_stdc_bit_floor", 0x15, None, 8),
            Some(0x10)
        );
        assert_eq!(
            eval_stdc_bit_builtin("__builtin_stdc_bit_ceil", 0, None, 8),
            Some(1)
        );
        assert_eq!(
            eval_stdc_bit_builtin("__builtin_stdc_bit_ceil", 0x81, None, 8),
            Some(0)
        );
        assert_eq!(
            eval_stdc_bit_builtin("__builtin_stdc_rotate_left", 1, Some(-1), 8),
            Some(0x80)
        );
        assert_eq!(
            eval_stdc_bit_builtin("__builtin_stdc_rotate_right", 0x8000_0000, Some(-1), 32),
            Some(1)
        );
    }
}
