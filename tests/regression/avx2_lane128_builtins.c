/* 128-bit lane builtins under -mavx2.
 *
 * GCC 16.2 defines __builtin_ia32_extract128i256 and __builtin_ia32_insert128i256
 * (both take a one-bit lane selector). They must select the same lane as the
 * _mm256_ wrappers. Output is a checksum over both lanes after extract and
 * insert, so any lane mix-up or dropped result changes the printed value.
 *
 * Expected output is identical under GCC 16.2 and lccc (checksum below).
 */
#include <immintrin.h>
#include <stdio.h>
#include <string.h>

static unsigned fold(const void *p, unsigned n, unsigned h) {
    const unsigned char *b = (const unsigned char *)p;
    for (unsigned i = 0; i < n; i++)
        h = (h ^ b[i]) * 16777619u;
    return h;
}

int main(void) {
    __m256i a = _mm256_setr_epi32(10, 11, 12, 13, 14, 15, 16, 17);
    __m128i b = _mm_setr_epi32(90, 91, 92, 93);
    unsigned h = 2166136261u;

    __m128i lo0 = __builtin_ia32_extract128i256((__v4di)a, 0);
    __m128i hi1 = __builtin_ia32_extract128i256((__v4di)a, 1);
    __m128i hi1w = _mm256_extracti128_si256(a, 1);
    h = fold(&lo0, sizeof lo0, h);
    h = fold(&hi1, sizeof hi1, h);
    h = fold(&hi1w, sizeof hi1w, h);

    __m256i ins0 = __builtin_ia32_insert128i256(a, b, 0);
    __m256i ins1 = __builtin_ia32_insert128i256(a, b, 1);
    __m256i ins1w = _mm256_inserti128_si256(a, b, 1);
    h = fold(&ins0, sizeof ins0, h);
    h = fold(&ins1, sizeof ins1, h);
    h = fold(&ins1w, sizeof ins1w, h);

    int lanes[8];
    memcpy(lanes, &ins1, sizeof lanes);
    printf("%d %d %d %d | %d %d %d %d\n", lanes[0], lanes[1], lanes[2], lanes[3],
           lanes[4], lanes[5], lanes[6], lanes[7]);
    printf("%08x\n", h);
    return 0;
}
