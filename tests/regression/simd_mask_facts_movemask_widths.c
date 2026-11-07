/* All three 128-bit movemask widths: byte (16 lanes), float (4 x 32-bit
 * sign bits) and double (2 x 64-bit sign bits).  Every image is constant;
 * GCC 16.2 and Clang 23.1.5 agree on the three immediates. */
#include <emmintrin.h>
#include <stdio.h>
int main(void) {
    __m128i a = _mm_setr_epi8(
        -1, 0, -1, -1, 0, 0, -1, 0,
        -1, -1, -1, -1, 0, 0, -1, -1);
    __m128  f = _mm_setr_ps(-0.0f, 1.0f, -0.0f, 1.0f);
    __m128d d = _mm_setr_pd(-0.0, 1.0);
    printf("%d %d %d\n", _mm_movemask_epi8(a), _mm_movemask_ps(f),
           _mm_movemask_pd(d));
    return 0;
}
