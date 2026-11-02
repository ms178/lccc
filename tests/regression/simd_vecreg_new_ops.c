/* Register-resident SSE values across operations added after the original
 * vecreg whitelist.  sat_chain_store deliberately uses t and u more than
 * once; without RA-21 they spill to stack between packed operations. */
#include <immintrin.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

__attribute__((noinline))
void sat_chain_store(const uint8_t *a, const uint8_t *b,
                     const uint8_t *c, uint8_t *out) {
    __m128i va = _mm_loadu_si128((const __m128i *)a);
    __m128i vb = _mm_loadu_si128((const __m128i *)b);
    __m128i vc = _mm_loadu_si128((const __m128i *)c);
    __m128i t = _mm_adds_epu8(va, vb);
    __m128i u = _mm_adds_epu8(t, vc);
    __m128i v = _mm_avg_epu8(t, u);
    __m128i w = _mm_sub_epi64(v, t);
    _mm_storeu_si128((__m128i *)out, _mm_xor_si128(w, u));
}

static void scalar_reference(const uint8_t *a, const uint8_t *b,
                             const uint8_t *c, uint8_t *expect) {
    uint8_t t[16], u[16], v[16];
    uint64_t t64[2], v64[2], w64[2];
    for (int i = 0; i < 16; ++i) {
        unsigned ab = (unsigned)a[i] + b[i];
        t[i] = (uint8_t)(ab > 255 ? 255 : ab);
        unsigned tc = (unsigned)t[i] + c[i];
        u[i] = (uint8_t)(tc > 255 ? 255 : tc);
        v[i] = (uint8_t)(((unsigned)t[i] + u[i] + 1) >> 1);
    }
    memcpy(t64, t, sizeof t64);
    memcpy(v64, v, sizeof v64);
    for (int lane = 0; lane < 2; ++lane)
        w64[lane] = v64[lane] - t64[lane];
    memcpy(expect, w64, 16);
    for (int i = 0; i < 16; ++i)
        expect[i] ^= u[i];
}

int main(void) {
    uint8_t a[16], b[16], c[16], out[16], expect[16];
    uint32_t random = 0x21a46u;

    for (int round = 0; round < 512; ++round) {
        for (int i = 0; i < 16; ++i) {
            random = random * 1664525u + 1013904223u;
            a[i] = (uint8_t)(random >> 24);
            random = random * 1664525u + 1013904223u;
            b[i] = (uint8_t)(random >> 24);
            random = random * 1664525u + 1013904223u;
            c[i] = (uint8_t)(random >> 24);
        }
        scalar_reference(a, b, c, expect);
        sat_chain_store(a, b, c, out);
        for (int i = 0; i < 16; ++i) {
            if (out[i] != expect[i]) {
                fprintf(stderr,
                        "round %d lane %d: got %u expected %u\n",
                        round, i, (unsigned)out[i], (unsigned)expect[i]);
                return 1;
            }
        }
    }

    /* Unary widening ops share the exact-argument classifier.  They used to
     * hit the binary emitter's two-argument assertion and panic the compiler. */
    uint16_t widened8[8];
    uint32_t widened16[4];
    __m128i va = _mm_loadu_si128((const __m128i *)a);
    __m128i z8 = _mm_cvtepu8_epi16(va);
    _mm_storeu_si128((__m128i *)widened8, z8);
    for (int i = 0; i < 8; ++i)
        if (widened8[i] != a[i])
            return 2;
    __m128i z16 = _mm_cvtepu16_epi32(z8);
    _mm_storeu_si128((__m128i *)widened16, z16);
    for (int i = 0; i < 4; ++i)
        if (widened16[i] != a[i])
            return 3;

    puts("OK simd_vecreg_new_ops");
    return 0;
}
