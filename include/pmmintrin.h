/* CCC compiler bundled pmmintrin.h - SSE3 intrinsics */
#ifndef _PMMINTRIN_H_INCLUDED
#define _PMMINTRIN_H_INCLUDED

/* SSE3 intrinsics are only available on x86/x86-64 targets */
#if !defined(__x86_64__) && !defined(__i386__) && !defined(__i686__)
#error "SSE3 intrinsics (pmmintrin.h) require an x86 target"
#endif

#include <emmintrin.h>

/* _mm_hadd_ps: horizontal add packed single-precision (HADDPS)
 * Result: { a[0]+a[1], a[2]+a[3], b[0]+b[1], b[2]+b[3] } */

/* _mm_hadd_pd: horizontal add packed double-precision (HADDPD)
 * Result: { a[0]+a[1], b[0]+b[1] } */

/* _mm_hsub_ps: horizontal subtract packed single-precision (HSUBPS) */

/* _mm_movedup_pd: duplicate the low double (MOVDDUP). GCC defines it as
 * _mm_shuffle_pd(x, x, 0), which is the same unpacklo of x with itself. */
static __inline__ __m128d __attribute__((__always_inline__))
_mm_movedup_pd(__m128d __a)
{
    return _mm_unpacklo_pd(__a, __a);
}

/* _mm_loaddup_pd: load one double and duplicate it into both lanes (MOVDDUP
 * with a memory source). */
static __inline__ __m128d __attribute__((__always_inline__, target("sse3")))
_mm_loaddup_pd(double const *__p)
{
    return _mm_set1_pd(*__p);
}

/* _mm_lddqu_si128: unaligned 128-bit load (LDDQU). LDDQU is a performance
 * hint for loads that cross cache lines; the value is that of an unaligned
 * load, which is what this implementation performs. */
static __inline__ __m128i __attribute__((__always_inline__, target("sse3")))
_mm_lddqu_si128(__m128i const *__p)
{
    return _mm_loadu_si128(__p);
}

/* _mm_monitor / _mm_mwait: arm and wait on the address range (MONITOR, MWAIT).
 * Both are privileged or OS-gated, so they are only valid where the OS allows
 * them. The operands follow the SDM: EAX = address, ECX = extensions,
 * EDX = hints. */
static __inline__ void __attribute__((__always_inline__, target("sse3")))
_mm_monitor(void const *__p, unsigned int __extensions, unsigned int __hints)
{
    __asm__ __volatile__("monitor" : : "a"(__p), "c"(__extensions), "d"(__hints));
}

static __inline__ void __attribute__((__always_inline__, target("sse3")))
_mm_mwait(unsigned int __extensions, unsigned int __hints)
{
    __asm__ __volatile__("mwait" : : "a"(__extensions), "c"(__hints));
}

/* _mm_movehdup_ps: duplicate odd-indexed single-precision elements (MOVSHDUP) */

/* _mm_moveldup_ps: duplicate even-indexed single-precision elements (MOVSLDUP) */

#endif /* _PMMINTRIN_H_INCLUDED */
