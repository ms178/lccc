/* Regression: variable-index 256-bit permute (vpermilps/vpermilpd, 0F38.0C/0D).
 * The builtin was declared in the header but had no backend lowering, and the
 * assembler only knew the immediate form (0F3A.04/05). */
#include <immintrin.h>
#include <stdio.h>

int main(void) {
    __m256 a = _mm256_set_ps(8, 7, 6, 5, 4, 3, 2, 1);
    __m256i idx = _mm256_set_epi32(0, 1, 2, 3, 4, 5, 6, 7); /* reverse */
    __m256 r = _mm256_permutevar_ps(a, idx);
    float out[8];
    _mm256_storeu_ps(out, r);
    float exp[8] = {4, 3, 2, 1, 8, 7, 6, 5}; /* vpermilps is per-128-bit lane */
    for (int i = 0; i < 8; i++)
        if (out[i] != exp[i]) {
            printf("FAIL lane %d: got %.0f expect %.0f\n", i, out[i], exp[i]);
            return 1;
        }
    printf("OK\n");
    return 0;
}
