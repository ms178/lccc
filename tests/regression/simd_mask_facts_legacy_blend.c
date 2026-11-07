/* A well-formed ALL-ON dword selector (0x80000000 per lane) has don't-care
 * low bytes.  `vblendvps` keys only on bit 31 of each dword, so the whole
 * blend is exactly the true arm.  GCC 16.2 and Clang 23.1.5 emit no
 * blendvps at all for this program. */
#include <smmintrin.h>
#include <stdio.h>
int main(void) {
    __m128 a = _mm_setr_ps(1.0f, 2.0f, 3.0f, 4.0f);
    __m128 b = _mm_setr_ps(5.0f, 6.0f, 7.0f, 8.0f);
    __m128 m = _mm_castsi128_ps(_mm_set1_epi32((int)0x80000000u));
    __m128 r = _mm_blendv_ps(a, b, m);
    printf("%g %g %g %g\n", r[0], r[1], r[2], r[3]);
    return 0;
}
