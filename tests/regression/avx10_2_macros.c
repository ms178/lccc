/* AVX10.2 predefined-macro contract (measured against gcc-16.2 `-dM -E`):
 * -mavx10.2 defines __AVX10_2__ in addition to everything -mavx10.1 defines,
 * and requires AVX10.1 (so __AVX10_1__ is also defined). No FMA. */
#if !defined(__AVX10_1__) || !defined(__AVX10_2__)
#error "-mavx10.2 must define both __AVX10_1__ and __AVX10_2__"
#endif
#if !defined(__AVX512F__) || !defined(__AVX512VL__) || !defined(__AVX512BW__) || \
    !defined(__AVX512DQ__) || !defined(__AVX512CD__)
#error "-mavx10.2 implies the AVX-512 base set"
#endif
#if defined(__FMA__)
#error "-mavx10.2 must not imply FMA"
#endif

#include <stdio.h>

int main(void)
{
    printf("avx10_2 macros ok\n");
    return 0;
}
