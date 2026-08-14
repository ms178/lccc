/* SSE2 arithmetic intrinsics vs scalar reference.
 * Requires: -msse2 (default on x86-64). Verifies exact lane semantics. */
#include <immintrin.h>
#include <stdint.h>
#include <stdio.h>

int main(void) {
    int16_t a16[8] = {100, 200, 300, 400, -100, -200, -300, -400};
    int16_t b16[8] = {10, 20, 30, 40, 50, 60, 70, 80};
    int32_t a32[4] = {100000, 200000, -300000, 400000};
    int32_t b32[4] = {1000, 2000, 3000, 4000};
    uint8_t a8[16], b8[16];
    for (int i = 0; i < 16; i++) { a8[i] = (uint8_t)(i * 13); b8[i] = (uint8_t)(i * 7 + 1); }

    __m128i va = _mm_loadu_si128((const __m128i*)a16);
    __m128i vb = _mm_loadu_si128((const __m128i*)b16);
    __m128i v32a = _mm_loadu_si128((const __m128i*)a32);
    __m128i v32b = _mm_loadu_si128((const __m128i*)b32);
    __m128i v8a = _mm_loadu_si128((const __m128i*)a8);
    __m128i v8b = _mm_loadu_si128((const __m128i*)b8);

    /* paddw / psubw */
    int16_t r16[8];
    _mm_storeu_si128((__m128i*)r16, _mm_add_epi16(va, vb));
    for (int i = 0; i < 8; i++) if (r16[i] != (int16_t)(a16[i] + b16[i])) return 1;
    _mm_storeu_si128((__m128i*)r16, _mm_sub_epi16(va, vb));
    for (int i = 0; i < 8; i++) if (r16[i] != (int16_t)(a16[i] - b16[i])) return 2;

    /* paddd */
    int32_t r32[4];
    _mm_storeu_si128((__m128i*)r32, _mm_add_epi32(v32a, v32b));
    for (int i = 0; i < 4; i++) if (r32[i] != a32[i] + b32[i]) return 3;
    _mm_storeu_si128((__m128i*)r32, _mm_mul_epu32(v32a, v32b)); /* low 32x32->64 */
    /* mul_epu32: multiplies even-indexed u32 lanes, result in 64-bit lanes */
    uint64_t r64[2];
    _mm_storeu_si128((__m128i*)r64, _mm_mul_epu32(v32a, v32b));
    if (r64[0] != (uint64_t)(uint32_t)a32[0] * (uint32_t)b32[0]) return 4;
    if (r64[1] != (uint64_t)(uint32_t)a32[2] * (uint32_t)b32[2]) return 5;

    /* pmullw / pmulhw */
    int16_t r16b[8];
    _mm_storeu_si128((__m128i*)r16b, _mm_mullo_epi16(va, vb));
    for (int i = 0; i < 8; i++) if (r16b[i] != (int16_t)(a16[i] * b16[i])) return 6;
    _mm_storeu_si128((__m128i*)r16b, _mm_mulhi_epi16(va, vb));
    for (int i = 0; i < 8; i++) if (r16b[i] != (int16_t)(((int)a16[i] * b16[i]) >> 16)) return 7;

    /* paddusb / psubusb (saturating) */
    uint8_t r8[16];
    _mm_storeu_si128((__m128i*)r8, _mm_adds_epu8(v8a, v8b));
    for (int i = 0; i < 16; i++) {
        int s = (int)a8[i] + b8[i];
        uint8_t want = s > 255 ? 255 : (uint8_t)s;
        if (r8[i] != want) return 8;
    }
    _mm_storeu_si128((__m128i*)r8, _mm_subs_epu8(v8a, v8b));
    for (int i = 0; i < 16; i++) {
        int d = (int)a8[i] - b8[i];
        uint8_t want = d < 0 ? 0 : (uint8_t)d;
        if (r8[i] != want) return 9;
    }

    /* pcmpeqb / pcmpgtb -> movemask */
    __m128i eq = _mm_cmpeq_epi8(v8a, v8a);
    if (_mm_movemask_epi8(eq) != 0xFFFF) return 10;
    __m128i gt = _mm_cmpgt_epi8(v8a, v8b);
    int mask = _mm_movemask_epi8(gt);
    for (int i = 0; i < 16; i++) {
        int want = (int8_t)a8[i] > (int8_t)b8[i] ? 1 : 0;
        if (((mask >> i) & 1) != want) return 11;
    }

    /* shifts */
    int16_t sh[8] = {1, 2, 4, 8, 16, 32, 64, 128};
    __m128i vs = _mm_loadu_si128((const __m128i*)sh);
    _mm_storeu_si128((__m128i*)r16, _mm_slli_epi16(vs, 3));
    for (int i = 0; i < 8; i++) if (r16[i] != sh[i] << 3) return 12;
    _mm_storeu_si128((__m128i*)r16, _mm_srli_epi16(vs, 2));
    for (int i = 0; i < 8; i++) if (r16[i] != sh[i] >> 2) return 13;

    /* set1 / set */
    __m128i ones = _mm_set1_epi32(0x01020304);
    int32_t ones_out[4];
    _mm_storeu_si128((__m128i*)ones_out, ones);
    for (int i = 0; i < 4; i++) if (ones_out[i] != 0x01020304) return 14;
    __m128i set = _mm_set_epi32(4, 3, 2, 1);
    _mm_storeu_si128((__m128i*)ones_out, set);
    if (ones_out[0] != 1 || ones_out[1] != 2 || ones_out[2] != 3 || ones_out[3] != 4) return 15;

    /* unpack / pack */
    int16_t lo[8] = {1,2,3,4,5,6,7,8}, hi[8] = {9,10,11,12,13,14,15,16};
    __m128i vlo = _mm_loadu_si128((const __m128i*)lo);
    __m128i vhi = _mm_loadu_si128((const __m128i*)hi);
    int16_t inter[8];
    _mm_storeu_si128((__m128i*)inter, _mm_unpacklo_epi16(vlo, vhi));
    if (inter[0]!=1 || inter[1]!=9 || inter[2]!=2 || inter[3]!=10) return 16;

    /* min/max */
    int16_t mn[8];
    _mm_storeu_si128((__m128i*)mn, _mm_min_epi16(va, vb));
    for (int i = 0; i < 8; i++) if (mn[i] != (a16[i] < b16[i] ? a16[i] : b16[i])) return 17;

    /* loadu/storeu round trip */
    uint8_t rt[16];
    _mm_storeu_si128((__m128i*)rt, v8a);
    for (int i = 0; i < 16; i++) if (rt[i] != a8[i]) return 18;

    printf("OK simd_sse2_arith\n");
    return 0;
}
