/* Regression: _mm_blendv_epi8 must use the THIRD operand (mask) as the
 * implicit PBLENDVB mask register (XMM0). The generic SSE binary-op path
 * previously dropped the mask and used the first operand's register as the
 * implicit mask, miscompiling zlib-ng's AVX2 inflate GET_CHUNK_MAG (the
 * dist>=16 blend between 128-bit lanes produced wrong bytes beyond byte 16).
 */
#include <immintrin.h>
#include <stdint.h>

int main(void) {
    uint8_t a[16], b[16], mask[16], out[16];
    for (int i = 0; i < 16; ++i) {
        a[i] = (uint8_t)(0x10 + i);     /* selected when mask bit clear */
        b[i] = (uint8_t)(0x40 + i);     /* selected when mask bit set   */
        mask[i] = (i & 1) ? 0x80u : 0u; /* MSB set for odd bytes        */
    }
    __m128i va = _mm_loadu_si128((const __m128i *)a);
    __m128i vb = _mm_loadu_si128((const __m128i *)b);
    __m128i vm = _mm_loadu_si128((const __m128i *)mask);
    _mm_storeu_si128((__m128i *)out, _mm_blendv_epi8(va, vb, vm));

    for (int i = 0; i < 16; ++i) {
        uint8_t want = (mask[i] & 0x80u) ? b[i] : a[i];
        if (out[i] != want)
            return 1; /* byte i: got out[i], want want */
    }

    /* Second pattern: mask 0xFF everywhere -> all of b; 0x00 -> all of a. */
    __m128i vone = _mm_set1_epi8(0xFF);
    __m128i vzero = _mm_set1_epi8(0x00);
    _mm_storeu_si128((__m128i *)out, _mm_blendv_epi8(va, vb, vone));
    for (int i = 0; i < 16; ++i) if (out[i] != b[i]) return 2;
    _mm_storeu_si128((__m128i *)out, _mm_blendv_epi8(va, vb, vzero));
    for (int i = 0; i < 16; ++i) if (out[i] != a[i]) return 3;

    return 0;
}
