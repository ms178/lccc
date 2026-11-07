/* movemask(blend(const, const, all-on dword selector)) must fold to one
 * immediate: exercises the computed-vector object-image proof. */
#include <smmintrin.h>
#include <stdio.h>
int main(void) {
    __m128 a = _mm_setr_ps(1.0f, 2.0f, 3.0f, 4.0f);
    __m128 b = _mm_setr_ps(-0.0f, -0.0f, -0.0f, -0.0f);
    __m128 m = _mm_castsi128_ps(_mm_set1_epi32((int)0x80000000u));
    printf("%d\n", _mm_movemask_ps(_mm_blendv_ps(a, b, m)));
    return 0;
}
