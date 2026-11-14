/* CCC compiler bundled fmaintrin.h - FMA3 intrinsics */
#ifndef _FMAINTRIN_H_INCLUDED
#define _FMAINTRIN_H_INCLUDED

#include <avxintrin.h>

/* Every FMA3 intrinsic is defined in <lcccsimd.h> (pulled in by
 * <xmmintrin.h>), next to the native helpers that carry them:
 *
 *   packed 128-bit : _mm_{fmadd,fmsub,fnmadd,fnmsub,fmaddsub,fmsubadd}_{ps,pd}
 *   scalar 128-bit : _mm_{fmadd,fmsub,fnmadd,fnmsub}_{ss,sd}
 *   packed 256-bit : _mm256_{fmadd,fmsub,fnmadd,fnmsub,fmaddsub,fmsubadd}_{ps,pd}
 *
 * They are NOT defined here, and in particular NOT as lane loops.  The whole
 * point of an FMA intrinsic is that the product is rounded exactly once, after
 * the addend is applied; a loop over `__a[__i] * __b[__i] +- __c[__i]` is two
 * roundings and is only correct when -ffp-contract=fast (the default)
 * contracts the loop back into one instruction.  Under -ffp-contract=off that
 * loop returned 0 for `_mm256_fmsub_ps(1.0000001, 1-1.1920929e-7, 1)` where
 * gcc-16, clang-23 and icx all return -1.42108547e-14, because the fused form
 * keeps the bits the rounded product discards.  An intrinsic whose contract is
 * "fused" must not depend on a compiler option, so each of these is a call to
 * a native helper that emits the single instruction at every optimization
 * level and under every -ffp-contract setting.
 *
 * The 512-bit spellings live in <avx512fintrin.h> for the same reason.
 */

#endif /* _FMAINTRIN_H_INCLUDED */
