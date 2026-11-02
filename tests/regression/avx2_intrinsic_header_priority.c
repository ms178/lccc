/*
 * Regression: compiler-owned intrinsic headers must win over a user -I GCC
 * include directory.  GCC 14's umbrella immintrin.h pulls AVX-512/_Float16
 * declarations that are irrelevant to an AVX2 probe and outside LCCC's current
 * frontend surface.  This must still compile and execute as ordinary AVX2.
 */
#include <immintrin.h>
#include <stdio.h>

int main(void) {
    __m256i a = _mm256_set1_epi32(17);
    __m256i b = _mm256_add_epi32(a, a);
    int lane = _mm256_extract_epi32(b, 0);
    printf("%d\n", lane);
    return lane == 34 ? 0 : 1;
}
