/* ISO C23 / C2y 7.18 <stdbit.h> — Bit and byte utilities for LCCC. */
#ifndef _LCCC_STDBIT_H
#define _LCCC_STDBIT_H

#define __STDC_VERSION_STDBIT_H__ 202311L

#ifndef __STDC_ENDIAN_LITTLE__
#ifdef __ORDER_LITTLE_ENDIAN__
#define __STDC_ENDIAN_LITTLE__ __ORDER_LITTLE_ENDIAN__
#else
#define __STDC_ENDIAN_LITTLE__ 1234
#endif
#endif

#ifndef __STDC_ENDIAN_BIG__
#ifdef __ORDER_BIG_ENDIAN__
#define __STDC_ENDIAN_BIG__ __ORDER_BIG_ENDIAN__
#else
#define __STDC_ENDIAN_BIG__ 4321
#endif
#endif

#ifndef __STDC_ENDIAN_NATIVE__
#ifdef __BYTE_ORDER__
#define __STDC_ENDIAN_NATIVE__ __BYTE_ORDER__
#else
#define __STDC_ENDIAN_NATIVE__ __STDC_ENDIAN_LITTLE__
#endif
#endif

#define __LCCC_STDBIT_UNARY_FAMILY(__base, __ret_uc, __ret_us, __ret_ui, __ret_ul, __ret_ull) \
    static __inline __ret_uc __base##_uc(unsigned char __x) { return __builtin_##__base(__x); } \
    static __inline __ret_us __base##_us(unsigned short __x) { return __builtin_##__base(__x); } \
    static __inline __ret_ui __base##_ui(unsigned int __x) { return __builtin_##__base(__x); } \
    static __inline __ret_ul __base##_ul(unsigned long __x) { return __builtin_##__base(__x); } \
    static __inline __ret_ull __base##_ull(unsigned long long __x) { return __builtin_##__base(__x); }

__LCCC_STDBIT_UNARY_FAMILY(stdc_leading_zeros, unsigned int, unsigned int, unsigned int, unsigned int, unsigned int)
__LCCC_STDBIT_UNARY_FAMILY(stdc_leading_ones, unsigned int, unsigned int, unsigned int, unsigned int, unsigned int)
__LCCC_STDBIT_UNARY_FAMILY(stdc_trailing_zeros, unsigned int, unsigned int, unsigned int, unsigned int, unsigned int)
__LCCC_STDBIT_UNARY_FAMILY(stdc_trailing_ones, unsigned int, unsigned int, unsigned int, unsigned int, unsigned int)
__LCCC_STDBIT_UNARY_FAMILY(stdc_first_leading_zero, unsigned int, unsigned int, unsigned int, unsigned int, unsigned int)
__LCCC_STDBIT_UNARY_FAMILY(stdc_first_leading_one, unsigned int, unsigned int, unsigned int, unsigned int, unsigned int)
__LCCC_STDBIT_UNARY_FAMILY(stdc_first_trailing_zero, unsigned int, unsigned int, unsigned int, unsigned int, unsigned int)
__LCCC_STDBIT_UNARY_FAMILY(stdc_first_trailing_one, unsigned int, unsigned int, unsigned int, unsigned int, unsigned int)
__LCCC_STDBIT_UNARY_FAMILY(stdc_count_zeros, unsigned int, unsigned int, unsigned int, unsigned int, unsigned int)
__LCCC_STDBIT_UNARY_FAMILY(stdc_count_ones, unsigned int, unsigned int, unsigned int, unsigned int, unsigned int)
__LCCC_STDBIT_UNARY_FAMILY(stdc_has_single_bit, _Bool, _Bool, _Bool, _Bool, _Bool)
__LCCC_STDBIT_UNARY_FAMILY(stdc_bit_width, unsigned int, unsigned int, unsigned int, unsigned int, unsigned int)
__LCCC_STDBIT_UNARY_FAMILY(stdc_bit_floor, unsigned char, unsigned short, unsigned int, unsigned long, unsigned long long)
__LCCC_STDBIT_UNARY_FAMILY(stdc_bit_ceil, unsigned char, unsigned short, unsigned int, unsigned long, unsigned long long)

#undef __LCCC_STDBIT_UNARY_FAMILY

static __inline unsigned char stdc_rotate_left_uc(unsigned char __x, int __c) { return __builtin_stdc_rotate_left(__x, __c); }
static __inline unsigned short stdc_rotate_left_us(unsigned short __x, int __c) { return __builtin_stdc_rotate_left(__x, __c); }
static __inline unsigned int stdc_rotate_left_ui(unsigned int __x, int __c) { return __builtin_stdc_rotate_left(__x, __c); }
static __inline unsigned long stdc_rotate_left_ul(unsigned long __x, int __c) { return __builtin_stdc_rotate_left(__x, __c); }
static __inline unsigned long long stdc_rotate_left_ull(unsigned long long __x, int __c) { return __builtin_stdc_rotate_left(__x, __c); }

static __inline unsigned char stdc_rotate_right_uc(unsigned char __x, int __c) { return __builtin_stdc_rotate_right(__x, __c); }
static __inline unsigned short stdc_rotate_right_us(unsigned short __x, int __c) { return __builtin_stdc_rotate_right(__x, __c); }
static __inline unsigned int stdc_rotate_right_ui(unsigned int __x, int __c) { return __builtin_stdc_rotate_right(__x, __c); }
static __inline unsigned long stdc_rotate_right_ul(unsigned long __x, int __c) { return __builtin_stdc_rotate_right(__x, __c); }
static __inline unsigned long long stdc_rotate_right_ull(unsigned long long __x, int __c) { return __builtin_stdc_rotate_right(__x, __c); }

#define stdc_leading_zeros(x) __builtin_stdc_leading_zeros(x)
#define stdc_leading_ones(x) __builtin_stdc_leading_ones(x)
#define stdc_trailing_zeros(x) __builtin_stdc_trailing_zeros(x)
#define stdc_trailing_ones(x) __builtin_stdc_trailing_ones(x)
#define stdc_first_leading_zero(x) __builtin_stdc_first_leading_zero(x)
#define stdc_first_leading_one(x) __builtin_stdc_first_leading_one(x)
#define stdc_first_trailing_zero(x) __builtin_stdc_first_trailing_zero(x)
#define stdc_first_trailing_one(x) __builtin_stdc_first_trailing_one(x)
#define stdc_count_zeros(x) __builtin_stdc_count_zeros(x)
#define stdc_count_ones(x) __builtin_stdc_count_ones(x)
#define stdc_has_single_bit(x) __builtin_stdc_has_single_bit(x)
#define stdc_bit_width(x) __builtin_stdc_bit_width(x)
#define stdc_bit_floor(x) __builtin_stdc_bit_floor(x)
#define stdc_bit_ceil(x) __builtin_stdc_bit_ceil(x)
#define stdc_rotate_left(x, c) __builtin_stdc_rotate_left((x), (c))
#define stdc_rotate_right(x, c) __builtin_stdc_rotate_right((x), (c))

#endif /* _LCCC_STDBIT_H */
