/* CCC compiler bundled bmi2intrin.h - BMI2 intrinsics */
#ifndef _BMI2INTRIN_H_INCLUDED
#define _BMI2INTRIN_H_INCLUDED

/* _bzhi_u32: zero high bits starting from specified bit position */
static __inline__ unsigned int __attribute__((__always_inline__))
_bzhi_u32(unsigned int __src, unsigned int __index)
{
#if defined(__x86_64__) && defined(__BMI2__)
    /* One `bzhi`; the IR operation folds and simplifies exactly. */
    return __builtin_ia32_bzhi_si(__src, __index);
#else
    __index &= 0xFF;
    if (__index >= 32)
        return __src;
    return __src & ((1U << __index) - 1U);
#endif
}

#ifdef __x86_64__
/* _bzhi_u64: zero high bits starting from specified bit position (64-bit) */
static __inline__ unsigned long long __attribute__((__always_inline__))
_bzhi_u64(unsigned long long __src, unsigned long long __index)
{
#ifdef __BMI2__
    return __builtin_ia32_bzhi_di(__src, __index);
#else
    __index &= 0xFF;
    if (__index >= 64)
        return __src;
    return __src & ((1ULL << __index) - 1ULL);
#endif
}
#endif

/* PDEP/PEXT. On x86-64 with BMI2 the builtins reach the backend, which
 * emits `pdep`/`pext` for each function that may use BMI2 and an exact
 * portable loop for one that may not: this header is compiled under the
 * translation unit's ISA and then inlined, so a caller's own
 * `target("arch=x86-64")` must not inherit an instruction its CPU lacks.
 * Elsewhere (no BMI2, i686) the portable form below is used. It visits
 * only the mask's set bits, without data-dependent branches. */
static __inline__ unsigned int __attribute__((__always_inline__))
__lccc_pdep_u32(unsigned int __src, unsigned int __mask)
{
    unsigned int __r = 0;
    while (__mask) {
        unsigned int __low = __mask & (0U - __mask);
        __r |= __low & (0U - (__src & 1U));
        __src >>= 1;
        __mask ^= __low;
    }
    return __r;
}

static __inline__ unsigned int __attribute__((__always_inline__))
__lccc_pext_u32(unsigned int __src, unsigned int __mask)
{
    unsigned int __r = 0, __bit = 1;
    while (__mask) {
        unsigned int __low = __mask & (0U - __mask);
        __r |= __bit & (0U - (unsigned int)((__src & __low) != 0));
        __bit <<= 1;
        __mask ^= __low;
    }
    return __r;
}

/* _pdep_u32: parallel bit deposit */
static __inline__ unsigned int __attribute__((__always_inline__))
_pdep_u32(unsigned int __src, unsigned int __mask)
{
#if defined(__x86_64__) && defined(__BMI2__)
    return __builtin_ia32_pdep_si(__src, __mask);
#else
    return __lccc_pdep_u32(__src, __mask);
#endif
}

/* _pext_u32: parallel bit extract */
static __inline__ unsigned int __attribute__((__always_inline__))
_pext_u32(unsigned int __src, unsigned int __mask)
{
#if defined(__x86_64__) && defined(__BMI2__)
    return __builtin_ia32_pext_si(__src, __mask);
#else
    return __lccc_pext_u32(__src, __mask);
#endif
}

#ifdef __x86_64__
/* _pdep_u64: parallel bit deposit (64-bit) */
static __inline__ unsigned long long __attribute__((__always_inline__))
_pdep_u64(unsigned long long __src, unsigned long long __mask)
{
#ifdef __BMI2__
    return __builtin_ia32_pdep_di(__src, __mask);
#else
    unsigned long long __r = 0;
    while (__mask) {
        unsigned long long __low = __mask & (0ULL - __mask);
        __r |= __low & (0ULL - (__src & 1ULL));
        __src >>= 1;
        __mask ^= __low;
    }
    return __r;
#endif
}

/* _pext_u64: parallel bit extract (64-bit) */
static __inline__ unsigned long long __attribute__((__always_inline__))
_pext_u64(unsigned long long __src, unsigned long long __mask)
{
#ifdef __BMI2__
    return __builtin_ia32_pext_di(__src, __mask);
#else
    unsigned long long __r = 0, __bit = 1;
    while (__mask) {
        unsigned long long __low = __mask & (0ULL - __mask);
        __r |= __bit & (0ULL - (unsigned long long)((__src & __low) != 0));
        __bit <<= 1;
        __mask ^= __low;
    }
    return __r;
#endif
}
#endif

/* _mulx_u32/_mulx_u64: full unsigned product, high half through the
 * pointer. A widening multiply is the exact operation; it needs no ISA
 * extension, so every function may use it. */
static __inline__ unsigned int __attribute__((__always_inline__))
_mulx_u32(unsigned int __x, unsigned int __y, unsigned int *__hi)
{
    unsigned long long __p = (unsigned long long)__x * __y;
    *__hi = (unsigned int)(__p >> 32);
    return (unsigned int)__p;
}

#ifdef __x86_64__
static __inline__ unsigned long long __attribute__((__always_inline__))
_mulx_u64(unsigned long long __x, unsigned long long __y, unsigned long long *__hi)
{
    unsigned __int128 __p = (unsigned __int128)__x * __y;
    *__hi = (unsigned long long)(__p >> 64);
    return (unsigned long long)__p;
}
#endif

#endif /* _BMI2INTRIN_H_INCLUDED */
