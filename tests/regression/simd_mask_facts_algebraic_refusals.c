/* The COMPLEMENT of every absorbing rule must NOT fold.  `and(v,-1)`,
 * `or(v,0)` and `xor(v,0)` all equal `v`, whose mask depends on `argc`.
 * A too-eager absorbing rule would print a constant here and diverge from
 * the GCC 16.2 / Clang 23.1.5 oracle.  Also covers the 256-bit forms. */
#include <emmintrin.h>
#include <immintrin.h>
#include <stdio.h>

int main(int argc, char **argv) {
    (void)argv;
    __m128i v = _mm_set1_epi8((char)(argc + 0x7F));
    __m128i z = _mm_setzero_si128();
    __m128i o = _mm_set1_epi8((char)-1);
    int a = _mm_movemask_epi8(_mm_and_si128(v, o));
    int b = _mm_movemask_epi8(_mm_or_si128(v, z));
    int c = _mm_movemask_epi8(_mm_xor_si128(v, z));
    int d = _mm_movemask_epi8(_mm_xor_si128(v, v));
    int e = _mm_movemask_epi8(_mm_cmpeq_epi8(v, v));
    printf("%d %d %d %d %d\n", a, b, c, d, e);

    __m256i w = _mm256_set1_epi8((char)(argc + 0x7F));
    int f = _mm256_movemask_epi8(_mm256_xor_si256(w, w));
    int g = _mm256_movemask_epi8(_mm256_cmpeq_epi8(w, w));
    int h = _mm256_movemask_epi8(_mm256_and_si256(w, _mm256_setzero_si256()));
    printf("%d %d %d\n", f, g, h);
    return 0;
}
