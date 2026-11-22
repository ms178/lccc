/* AVX10.1 predefined-macro contract (measured against gcc-16.2 `-dM -E`):
 * -mavx10.1 defines __AVX10_1__ and the AVX-512 base set, AVX2 and AVX, and
 * does NOT define __AVX10_2__ or __FMA__ (FMA is not implied). A violation is
 * a compile-time #error, so the suite fails on the build, not at run time. */
#if !defined(__AVX10_1__)
#error "-mavx10.1 must define __AVX10_1__"
#endif
#if defined(__AVX10_2__)
#error "-mavx10.1 must not define __AVX10_2__"
#endif
#if !defined(__AVX__) || !defined(__AVX2__)
#error "-mavx10.1 implies AVX and AVX2"
#endif
#if !defined(__AVX512F__) || !defined(__AVX512CD__) || !defined(__AVX512DQ__) || \
    !defined(__AVX512BW__) || !defined(__AVX512VL__) || !defined(__AVX512IFMA__) || \
    !defined(__AVX512VBMI__) || !defined(__AVX512VBMI2__) || !defined(__AVX512VNNI__) || \
    !defined(__AVX512BITALG__) || !defined(__AVX512VPOPCNTDQ__) || \
    !defined(__AVX512BF16__) || !defined(__AVX512FP16__)
#error "-mavx10.1 implies the AVX-512 base set"
#endif
#if defined(__FMA__) || defined(__GFNI__) || defined(__VAES__) || defined(__VPCLMULQDQ__)
#error "-mavx10.1 must not imply FMA, GFNI, VAES or VPCLMULQDQ"
#endif

#include <stdio.h>

int main(void)
{
    printf("avx10_1 macros ok\n");
    return 0;
}
