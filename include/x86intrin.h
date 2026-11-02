/* CCC compiler bundled x86intrin.h - x86 intrinsics umbrella header */
#ifndef _X86INTRIN_H_INCLUDED
#define _X86INTRIN_H_INCLUDED

/* x86 intrinsics are only available on x86/x86-64 targets */
#if !defined(__x86_64__) && !defined(__i386__) && !defined(__i686__)
#error "x86 intrinsics (x86intrin.h) require an x86 target"
#endif

/* Include all SIMD intrinsics */
#include <immintrin.h>

/* Scalar helpers (rdtsc, bit scans, rotates) live in ia32intrin.h, which
 * immintrin.h includes, matching GCC. */

#endif /* _X86INTRIN_H_INCLUDED */
