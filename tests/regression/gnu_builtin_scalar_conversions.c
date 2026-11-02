/* Intrinsic dispatch must perform C parameter conversions, not read FP bits. */
#include <immintrin.h>
#include <stdio.h>
#include <string.h>
static volatile double number = 1.75;
static volatile double bits = 63.0;
static volatile double count = 5.5;
int main(void) {
    double value = number;
    printf("%u %u %u\n", __builtin_ia32_crc32si(0, value),
           _mm_crc32_u32(0, value), __builtin_ia32_bzhi_si(bits, count));
    __m128 a = {7, 8, 9, 10};
    __m128 converted = _mm_cvtsi32_ss(a, value);
    printf("%.0f %.0f %.0f %.0f\n", (double)converted[0],
           (double)converted[1], (double)converted[2], (double)converted[3]);
#ifdef _LCCCSIMD_H_INCLUDED
    /* Exercise the generic helper dispatch as well as the public-name path. */
    __m128 private_value = __lccc_simd128_ps_cvtsi2ss128(0, a, value);
#else
    __m128 private_value = _mm_cvtsi32_ss(a, value);
#endif
    printf("%.0f %.0f\n", (double)private_value[0], (double)private_value[3]);
    unsigned char bytes[16];
    for (unsigned i = 0; i < sizeof bytes; ++i) bytes[i] = (unsigned char)(i + 1);
    __m128i vector;
    memcpy(&vector, bytes, sizeof vector);
    __m128i shifted = __builtin_ia32_pslldqi128(vector, 8.75);
    memcpy(bytes, &shifted, sizeof bytes);
    for (unsigned i = 0; i < sizeof bytes; ++i) printf("%02x", bytes[i]);
    puts("");
    __m128i shuffled = (__m128i)__builtin_ia32_pshufd((__v4si)vector, 3.75);
    memcpy(bytes, &shuffled, sizeof bytes);
    for (unsigned i = 0; i < sizeof bytes; ++i) printf("%02x", bytes[i]);
    puts("");
    return 0;
}
