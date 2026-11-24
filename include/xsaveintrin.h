/* CCC compiler bundled xsaveintrin.h - XSAVE, XRSTOR, XGETBV, XSETBV */
#ifndef _XSAVEINTRIN_H_INCLUDED
#define _XSAVEINTRIN_H_INCLUDED

/* The six intrinsics of GCC's xsaveintrin.h, each the instruction itself.
 *
 * _xgetbv is the one programs call: with index 0 it reads XCR0, the mask of
 * the register state the operating system saves and restores, which is how
 * runtime dispatch learns whether AVX or AVX-512 may be used at all (CPUID
 * says the processor has them, XCR0 says the system keeps them). It used to
 * be a stub returning 0: every caller concluded that nothing may be used. */

static __inline__ long long __attribute__((__always_inline__))
_xgetbv(unsigned int __index)
{
    unsigned int __lo, __hi;
    __asm__ __volatile__("xgetbv" : "=a"(__lo), "=d"(__hi) : "c"(__index));
    return (long long)(((unsigned long long)__hi << 32) | __lo);
}

/* Privileged: writes an extended control register (#GP outside ring 0). */
static __inline__ void __attribute__((__always_inline__))
_xsetbv(unsigned int __index, long long __value)
{
    __asm__ __volatile__("xsetbv"
                         :
                         : "c"(__index), "a"((unsigned int)__value),
                           "d"((unsigned int)((unsigned long long)__value >> 32)));
}

/* Save to, or restore from, the 64-byte-aligned area at __area the state
 * components selected by __mask (and enabled in XCR0). */
static __inline__ void __attribute__((__always_inline__))
_xsave(void *__area, long long __mask)
{
    __asm__ __volatile__("xsave (%0)"
                         :
                         : "r"(__area), "a"((unsigned int)__mask),
                           "d"((unsigned int)((unsigned long long)__mask >> 32))
                         : "memory");
}

static __inline__ void __attribute__((__always_inline__))
_xrstor(void *__area, long long __mask)
{
    __asm__ __volatile__("xrstor (%0)"
                         :
                         : "r"(__area), "a"((unsigned int)__mask),
                           "d"((unsigned int)((unsigned long long)__mask >> 32))
                         : "memory");
}

#ifdef __x86_64__
static __inline__ void __attribute__((__always_inline__))
_xsave64(void *__area, long long __mask)
{
    __asm__ __volatile__("xsave64 (%0)"
                         :
                         : "r"(__area), "a"((unsigned int)__mask),
                           "d"((unsigned int)((unsigned long long)__mask >> 32))
                         : "memory");
}

static __inline__ void __attribute__((__always_inline__))
_xrstor64(void *__area, long long __mask)
{
    __asm__ __volatile__("xrstor64 (%0)"
                         :
                         : "r"(__area), "a"((unsigned int)__mask),
                           "d"((unsigned int)((unsigned long long)__mask >> 32))
                         : "memory");
}
#endif

#endif /* _XSAVEINTRIN_H_INCLUDED */
