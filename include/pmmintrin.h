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

/* _mm_movehdup_ps: duplicate odd-indexed single-precision elements (MOVSHDUP) */

/* _mm_moveldup_ps: duplicate even-indexed single-precision elements (MOVSLDUP) */

#endif /* _PMMINTRIN_H_INCLUDED */
