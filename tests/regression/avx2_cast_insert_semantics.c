#include <immintrin.h>
#include <stdint.h>

int main(void) {
    uint8_t in[16], out[32];
    for (int i = 0; i < 16; ++i) in[i] = (uint8_t)(i * 7 + 3);
    __m128i x = _mm_loadu_si128((const __m128i *)in);

    _mm256_storeu_si256((__m256i_u *)out, _mm256_zextsi128_si256(x));
    for (int i = 0; i < 16; ++i) if (out[i] != in[i]) return 1;
    for (int i = 16; i < 32; ++i) if (out[i] != 0) return 2;

    __m256i zero = _mm256_setzero_si256();
    _mm256_storeu_si256((__m256i_u *)out, _mm256_inserti128_si256(zero, x, 0));
    for (int i = 0; i < 16; ++i) if (out[i] != in[i]) return 3;
    for (int i = 16; i < 32; ++i) if (out[i] != 0) return 4;

    _mm256_storeu_si256((__m256i_u *)out, _mm256_inserti128_si256(zero, x, 1));
    for (int i = 0; i < 16; ++i) if (out[i] != 0) return 5;
    for (int i = 16; i < 32; ++i) if (out[i] != in[i - 16]) return 6;

    _mm256_storeu_si256((__m256i_u *)out, _mm256_broadcastsi128_si256(x));
    for (int i = 0; i < 32; ++i) if (out[i] != in[i & 15]) return 7;
    return 0;
}
