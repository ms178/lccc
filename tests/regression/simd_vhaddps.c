/* Regression: vhaddps/vhsubps/vaddsubps use the F2 VEX prefix (pp=3).
 * They were encoded with the F3 prefix, an illegal instruction (SIGILL). */
#include <immintrin.h>
#include <stdio.h>

int main(void) {
    __m256 a = _mm256_set_ps(8, 7, 6, 5, 4, 3, 2, 1);
    __m256 b = _mm256_set_ps(16, 15, 14, 13, 12, 11, 10, 9);
    __m256 r = _mm256_hadd_ps(a, b);
    float out[8];
    _mm256_storeu_ps(out, r);
    float exp[8] = {3, 7, 19, 23, 11, 15, 27, 31};
    for (int i = 0; i < 8; i++)
        if (out[i] != exp[i]) {
            printf("FAIL lane %d: got %.0f expect %.0f\n", i, out[i], exp[i]);
            return 1;
        }
    printf("OK\n");
    return 0;
}
