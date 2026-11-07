/* Constant propagation through a legacy SSE byte-logic chain:
 * movemask(and(A, splat(0x80))) and movemask(cmpeq(A, splat(0x80))).
 * Both must fold to immediates; a wrong byte function is visible. */
#include <emmintrin.h>
#include <stdio.h>
int main(void) {
    __m128i a = _mm_setr_epi8(
        -1, 0, -1, -1, 0, 0, -1, 0,
        -1, -1, -1, -1, 0, 0, -1, -1);
    __m128i b = _mm_set1_epi8((char)0x80);
    int and_mask = _mm_movemask_epi8(_mm_and_si128(a, b));
    int eq_mask  = _mm_movemask_epi8(_mm_cmpeq_epi8(a, b));
    printf("%d %d\n", and_mask, eq_mask);
    return 0;
}
