/* LCCC bundled bmiintrin.h - BMI1 intrinsics (GCC/Clang names).
 *
 * Plain C whose IR folds exactly: under -mbmi the backend selects
 * tzcnt/andn/blsi/blsmsk/blsr; on baseline targets the same functions
 * return the architecturally defined BMI1 results. */
#ifndef _BMIINTRIN_H_INCLUDED
#define _BMIINTRIN_H_INCLUDED

/* TZCNT is defined at zero: the operand width. */
static __inline__ unsigned short __attribute__((__always_inline__))
__tzcnt_u16(unsigned short __x)
{
    return __x ? (unsigned short)__builtin_ctz(__x) : 16;
}

static __inline__ unsigned int __attribute__((__always_inline__))
__tzcnt_u32(unsigned int __x)
{
    return __x ? (unsigned int)__builtin_ctz(__x) : 32;
}

static __inline__ unsigned int __attribute__((__always_inline__))
_tzcnt_u32(unsigned int __x)
{
    return __tzcnt_u32(__x);
}

static __inline__ unsigned int __attribute__((__always_inline__))
__andn_u32(unsigned int __x, unsigned int __y)
{
    return ~__x & __y;
}

/* BEXTR: start = control[7:0], length = control[15:8]. A start of 32 or
 * more yields 0; a length of 32 or more keeps every bit above start. */
static __inline__ unsigned int __attribute__((__always_inline__))
__bextr_u32(unsigned int __x, unsigned int __control)
{
    unsigned int __start = __control & 0xFF;
    unsigned int __len = (__control >> 8) & 0xFF;
    if (__start >= 32)
        return 0;
    __x >>= __start;
    return __len >= 32 ? __x : __x & ((1U << __len) - 1U);
}

static __inline__ unsigned int __attribute__((__always_inline__))
_bextr_u32(unsigned int __x, unsigned int __start, unsigned int __len)
{
    return __bextr_u32(__x, (__start & 0xFF) | ((__len & 0xFF) << 8));
}

static __inline__ unsigned int __attribute__((__always_inline__))
__blsi_u32(unsigned int __x)
{
    return __x & -__x;
}

static __inline__ unsigned int __attribute__((__always_inline__))
_blsi_u32(unsigned int __x)
{
    return __blsi_u32(__x);
}

static __inline__ unsigned int __attribute__((__always_inline__))
__blsmsk_u32(unsigned int __x)
{
    return __x ^ (__x - 1U);
}

static __inline__ unsigned int __attribute__((__always_inline__))
_blsmsk_u32(unsigned int __x)
{
    return __blsmsk_u32(__x);
}

static __inline__ unsigned int __attribute__((__always_inline__))
__blsr_u32(unsigned int __x)
{
    return __x & (__x - 1U);
}

static __inline__ unsigned int __attribute__((__always_inline__))
_blsr_u32(unsigned int __x)
{
    return __blsr_u32(__x);
}

#ifdef __x86_64__
static __inline__ unsigned long long __attribute__((__always_inline__))
__tzcnt_u64(unsigned long long __x)
{
    return __x ? (unsigned long long)__builtin_ctzll(__x) : 64;
}

static __inline__ unsigned long long __attribute__((__always_inline__))
_tzcnt_u64(unsigned long long __x)
{
    return __tzcnt_u64(__x);
}

static __inline__ unsigned long long __attribute__((__always_inline__))
__andn_u64(unsigned long long __x, unsigned long long __y)
{
    return ~__x & __y;
}

static __inline__ unsigned long long __attribute__((__always_inline__))
__bextr_u64(unsigned long long __x, unsigned long long __control)
{
    unsigned int __start = (unsigned int)(__control & 0xFF);
    unsigned int __len = (unsigned int)((__control >> 8) & 0xFF);
    if (__start >= 64)
        return 0;
    __x >>= __start;
    return __len >= 64 ? __x : __x & ((1ULL << __len) - 1ULL);
}

static __inline__ unsigned long long __attribute__((__always_inline__))
_bextr_u64(unsigned long long __x, unsigned int __start, unsigned int __len)
{
    return __bextr_u64(__x, (__start & 0xFFULL) | ((__len & 0xFFULL) << 8));
}

static __inline__ unsigned long long __attribute__((__always_inline__))
__blsi_u64(unsigned long long __x)
{
    return __x & -__x;
}

static __inline__ unsigned long long __attribute__((__always_inline__))
_blsi_u64(unsigned long long __x)
{
    return __blsi_u64(__x);
}

static __inline__ unsigned long long __attribute__((__always_inline__))
__blsmsk_u64(unsigned long long __x)
{
    return __x ^ (__x - 1ULL);
}

static __inline__ unsigned long long __attribute__((__always_inline__))
_blsmsk_u64(unsigned long long __x)
{
    return __blsmsk_u64(__x);
}

static __inline__ unsigned long long __attribute__((__always_inline__))
__blsr_u64(unsigned long long __x)
{
    return __x & (__x - 1ULL);
}

static __inline__ unsigned long long __attribute__((__always_inline__))
_blsr_u64(unsigned long long __x)
{
    return __blsr_u64(__x);
}
#endif /* __x86_64__ */

#endif /* _BMIINTRIN_H_INCLUDED */
