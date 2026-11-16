/* Regression: the AVX1 broadcast-from-memory intrinsics and the 128-to-256
 * cast, which the bundled <immintrin.h> did not declare (a valid program
 * failed to compile). Each broadcast must return every lane as the exact bits
 * loaded from memory, including a NaN payload and negative zero, and the
 * 128-bit-source forms must repeat the 128-bit block into both halves.
 */
#include <immintrin.h>
#include <stdio.h>
#include <string.h>

static unsigned fold(unsigned acc, unsigned x) { return acc * 1000003u ^ x; }

static unsigned bits_of_f(__m256 v) {
    unsigned w[8], acc = 0;
    memcpy(w, &v, sizeof w);
    for (int i = 0; i < 8; ++i) acc = fold(acc, w[i]);
    return acc;
}

static unsigned bits_of_d(__m256d v) {
    unsigned long long w[4];
    unsigned acc = 0;
    memcpy(w, &v, sizeof w);
    for (int i = 0; i < 4; ++i) {
        acc = fold(acc, (unsigned)w[i]);
        acc = fold(acc, (unsigned)(w[i] >> 32));
    }
    return acc;
}

int main(void) {
    float f;
    unsigned fbits = 0x7fc12345u;               /* quiet NaN with payload */
    memcpy(&f, &fbits, sizeof f);
    double d = -0.0;
    float q[4] = {-0.0f, 2.0f, 3.0f, 4.0f};
    double dq[2] = {-0.0, 6.25};

    unsigned acc = 0;
    acc = fold(acc, bits_of_f(_mm256_broadcast_ss(&f)));
    acc = fold(acc, bits_of_d(_mm256_broadcast_sd(&d)));
    acc = fold(acc, bits_of_f(_mm256_broadcast_ps((const __m128 *)q)));
    acc = fold(acc, bits_of_d(_mm256_broadcast_pd((const __m128d *)dq)));
    /* The upper half of a 128-to-256 cast is unspecified: read the low four
     * lanes only. */
    __m128 low = _mm_loadu_ps(q);
    __m256 wide = _mm256_castps128_ps256(low);
    float wf[8];
    _mm256_storeu_ps(wf, wide);
    for (int i = 0; i < 4; ++i) {
        unsigned w;
        memcpy(&w, &wf[i], sizeof w);
        acc = fold(acc, w);
    }
    printf("avx_broadcast_intrinsics checksum=%u\n", acc);
    return 0;
}
