/* Regression: 256-bit vblendvps/vblendvpd result register.
 * The blend was computed into %ymm1 but avx_store_dest stores %ymm0, so the
 * mask operand was returned instead of the blended result. */
#include <immintrin.h>
#include <stdio.h>

int main(void) {
    __m256 a = _mm256_set_ps(8, 7, 6, 5, 4, 3, 2, 1);
    __m256 b = _mm256_set1_ps(2.0f);
    __m256 mask = _mm256_set_ps(-0.0f, 0.0f, -0.0f, 0.0f, -0.0f, 0.0f, -0.0f, 0.0f);
    __m256 r = _mm256_blendv_ps(a, b, mask);
    float out[8];
    _mm256_storeu_ps(out, r);
    float exp[8] = {1, 2, 3, 2, 5, 2, 7, 2};
    for (int i = 0; i < 8; i++)
        if (out[i] != exp[i]) {
            printf("FAIL lane %d: got %.0f expect %.0f\n", i, out[i], exp[i]);
            return 1;
        }
    printf("OK\n");
    return 0;
}
