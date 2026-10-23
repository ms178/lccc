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

/* _pdep_u32: parallel bit deposit */
static __inline__ unsigned int __attribute__((__always_inline__))
_pdep_u32(unsigned int __src, unsigned int __mask)
{
#ifdef __BMI2__
    unsigned int __r;
    __asm__("pdepl %2, %1, %0" : "=r"(__r) : "r"(__src), "rm"(__mask));
    return __r;
#else
    unsigned int __result = 0;
    unsigned int __k = 0;
    for (unsigned int __i = 0; __i < 32; __i++) {
        if (__mask & (1U << __i)) {
            if (__src & (1U << __k))
                __result |= (1U << __i);
            __k++;
        }
    }
    return __result;
#endif
}

/* _pext_u32: parallel bit extract */
static __inline__ unsigned int __attribute__((__always_inline__))
_pext_u32(unsigned int __src, unsigned int __mask)
{
#ifdef __BMI2__
    unsigned int __r;
    __asm__("pextl %2, %1, %0" : "=r"(__r) : "r"(__src), "rm"(__mask));
    return __r;
#else
    unsigned int __result = 0;
    unsigned int __k = 0;
    for (unsigned int __i = 0; __i < 32; __i++) {
        if (__mask & (1U << __i)) {
            if (__src & (1U << __i))
                __result |= (1U << __k);
            __k++;
        }
    }
    return __result;
#endif
}

#ifdef __x86_64__
/* _pdep_u64: parallel bit deposit (64-bit) */
static __inline__ unsigned long long __attribute__((__always_inline__))
_pdep_u64(unsigned long long __src, unsigned long long __mask)
{
#ifdef __BMI2__
    unsigned long long __r;
    __asm__("pdepq %2, %1, %0" : "=r"(__r) : "r"(__src), "rm"(__mask));
    return __r;
#else
    unsigned long long __result = 0;
    unsigned long long __k = 0;
    for (unsigned long long __i = 0; __i < 64; __i++) {
        if (__mask & (1ULL << __i)) {
            if (__src & (1ULL << __k))
                __result |= (1ULL << __i);
            __k++;
        }
    }
    return __result;
#endif
}

/* _pext_u64: parallel bit extract (64-bit) */
static __inline__ unsigned long long __attribute__((__always_inline__))
_pext_u64(unsigned long long __src, unsigned long long __mask)
{
#ifdef __BMI2__
    unsigned long long __r;
    __asm__("pextq %2, %1, %0" : "=r"(__r) : "r"(__src), "rm"(__mask));
    return __r;
#else
    unsigned long long __result = 0;
    unsigned long long __k = 0;
    for (unsigned long long __i = 0; __i < 64; __i++) {
        if (__mask & (1ULL << __i)) {
            if (__src & (1ULL << __i))
                __result |= (1ULL << __k);
            __k++;
        }
    }
    return __result;
#endif
}
#endif

#endif /* _BMI2INTRIN_H_INCLUDED */
