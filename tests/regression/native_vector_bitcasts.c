#include <emmintrin.h>
#if defined(__AVX__)
#include <immintrin.h>
#endif

__attribute__((noinline)) static void cast128(unsigned *out, const unsigned *in) {
    __m128 x = *(const __m128 *)(const void *)in;
    __m128i i = _mm_castps_si128(x);
    __m128d p = _mm_castps_pd(x);
    __m128 q = _mm_castpd_ps(p);
    __m128d d = _mm_castsi128_pd(i);
    __m128i j = _mm_castpd_si128(d);
    __m128 y = _mm_castsi128_ps(j);
    (void)q;
    *(__m128 *)(void *)out = y;
}

#if defined(__AVX__)
__attribute__((noinline)) static void cast256(unsigned *out, const unsigned *in) {
    __m256 x = *(const __m256 *)(const void *)in;
    __m256i i = _mm256_castps_si256(x);
    __m256d d = _mm256_castsi256_pd(i);
    __m256i j = _mm256_castpd_si256(d);
    __m256 y = _mm256_castsi256_ps(j);
    __m256d p = _mm256_castps_pd(y);
    __m256 z = _mm256_castpd_ps(p);
    *(__m256 *)(void *)out = z;
}
#endif

int main(void) {
    static const unsigned input[8] __attribute__((aligned(32))) = {
        0x80000000u, 0x7f812345u, 1u, 0xffc54321u,
        0x7f800000u, 0x00000001u, 0x3f800000u, 0xff800000u};
    unsigned output[8] __attribute__((aligned(32))) = {0};
    cast128(output, input);
    for (unsigned i = 0; i < 4; ++i) if (output[i] != input[i]) return 11;
#if defined(__AVX__)
    cast256(output, input);
    for (unsigned i = 0; i < 8; ++i) if (output[i] != input[i]) return 12;
#endif
    return 0;
}
