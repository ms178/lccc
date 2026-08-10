/* Regression: v5 deferred vector-store, multi-def slot soundness.
 *
 * zlib-ng's adler32_ssse3 miscompiled because the deferred-store analysis
 * keyed its decision by stack SLOT while soundness is per DEF SITE: the C
 * temporaries (v_sad_sum1 / vsum2) are written by BOTH the wide (k>=32) and
 * narrow (k>=16) loop bodies. In the narrow body the producer's single use
 * is the immediately-following intrinsic (store may be deferred: the value
 * flows through %xmm0); in the wide body another intrinsic overwrites %xmm0
 * before the consumer, so the store is mandatory there. The slot-level
 * decision deferred the store at BOTH sites, and the wide loop's consumer
 * read a never-written slot.
 *
 * This test reproduces that structure exactly: shared vector temporaries,
 * one loop body with adjacent producer->consumer, one with an intervening
 * producer. All results are checked against a scalar reference. */
#include <immintrin.h>
#include <stdint.h>
#include <stddef.h>
#include <string.h>

#define BASE 65521U

static uint32_t partial_hsum(__m128i x) {
    __m128i t = _mm_srli_si128(x, 8);
    return (uint32_t)_mm_cvtsi128_si32(_mm_add_epi32(x, t));
}

static uint32_t hsum(__m128i x) {
    __m128i s1 = _mm_unpackhi_epi64(x, x);
    __m128i s2 = _mm_add_epi32(x, s1);
    __m128i s3 = _mm_shuffle_epi32(s2, 0x01);
    return (uint32_t)_mm_cvtsi128_si32(_mm_add_epi32(s2, s3));
}

/* Adler-32 with the adler32_ssse3 dual-loop structure (aligned input). */
static uint32_t vec_adler(uint32_t adler, const uint8_t *buf, size_t len) {
    uint32_t sum2 = (adler >> 16) & 0xffff;
    adler &= 0xffff;

    const __m128i dot2v   = _mm_setr_epi8(32,31,30,29,28,27,26,25,24,23,22,21,20,19,18,17);
    const __m128i dot2v_0 = _mm_setr_epi8(16,15,14,13,12,11,10,9,8,7,6,5,4,3,2,1);
    const __m128i dot3v   = _mm_set1_epi16(1);
    const __m128i zero    = _mm_setzero_si128();

    __m128i vbuf, vbuf_0, vs1, vs2, vs1_0, vs3;
    /* Shared temporaries: written by BOTH loop bodies below. */
    __m128i v_sad_sum1, v_sad_sum2, v_short_sum2, v_short_sum2_0, vsum2, vsum2_0;

    size_t max_iters = 5552;
    size_t k = 0;

    while (len >= 16) {
        vs1 = _mm_cvtsi32_si128((int)adler);
        vs2 = _mm_cvtsi32_si128((int)sum2);
        vs3 = _mm_setzero_si128();
        __m128i vs2_0 = _mm_setzero_si128();
        vs1_0 = vs1;

        k = (len < max_iters ? len : max_iters);
        k -= k % 16;
        len -= k;

        /* Wide body: consumer of v_sad_sum1 / vsum2 is NOT adjacent
         * (v_sad_sum2 / v_short_sum2_0 producers intervene). */
        while (k >= 32) {
            vbuf   = _mm_load_si128((const __m128i *)buf);
            vbuf_0 = _mm_load_si128((const __m128i *)(buf + 16));
            buf += 32;
            k -= 32;

            __m128i sad1 = _mm_sad_epu8(vbuf, zero);
            __m128i sad2 = _mm_sad_epu8(vbuf_0, zero);
            vs1 = _mm_add_epi32(sad1, vs1);
            vs3 = _mm_add_epi32(vs1_0, vs3);
            vs1 = _mm_add_epi32(sad2, vs1);

            v_short_sum2   = _mm_maddubs_epi16(vbuf, dot2v);
            vsum2          = _mm_madd_epi16(v_short_sum2, dot3v);
            v_short_sum2_0 = _mm_maddubs_epi16(vbuf_0, dot2v_0);
            vs2 = _mm_add_epi32(vsum2, vs2);
            vsum2_0 = _mm_madd_epi16(v_short_sum2_0, dot3v);
            vs2_0 = _mm_add_epi32(vsum2_0, vs2_0);
            vs1_0 = vs1;
            v_sad_sum1 = sad1; v_sad_sum2 = sad2; /* keep values live */
        }

        vs2 = _mm_add_epi32(vs2_0, vs2);
        vs3 = _mm_slli_epi32(vs3, 5);
        vs2 = _mm_add_epi32(vs3, vs2);
        vs3 = _mm_setzero_si128();

        /* Narrow body: consumers ARE adjacent (deferral is legal here). */
        while (k >= 16) {
            vbuf = _mm_load_si128((const __m128i *)buf);
            buf += 16;
            k -= 16;

            v_sad_sum1 = _mm_sad_epu8(vbuf, zero);
            vs1 = _mm_add_epi32(v_sad_sum1, vs1);
            vs3 = _mm_add_epi32(vs1_0, vs3);
            v_short_sum2 = _mm_maddubs_epi16(vbuf, dot2v_0);
            vsum2 = _mm_madd_epi16(v_short_sum2, dot3v);
            vs2 = _mm_add_epi32(vsum2, vs2);
            vs1_0 = vs1;
        }

        vs3 = _mm_slli_epi32(vs3, 4);
        vs2 = _mm_add_epi32(vs2, vs3);

        adler = partial_hsum(vs1) % BASE;
        sum2 = hsum(vs2) % BASE;
        max_iters = 5552;
    }

    /* tail */
    while (len) {
        --len;
        adler += *buf++;
        sum2 += adler;
    }
    adler %= BASE;
    sum2 %= BASE;
    return adler | (sum2 << 16);
}

static uint32_t ref_adler(uint32_t adler, const uint8_t *buf, size_t len) {
    uint32_t s1 = adler & 0xffff, s2 = (adler >> 16) & 0xffff;
    for (size_t i = 0; i < len; i++) {
        s1 = (s1 + buf[i]) % BASE;
        s2 = (s2 + s1) % BASE;
    }
    return s1 | (s2 << 16);
}

int main(void) {
    static uint8_t buf[65536 + 64] __attribute__((aligned(64)));
    uint32_t st = 0x9E3779B9u;
    for (size_t i = 0; i < sizeof buf; i++) {
        st ^= st << 13; st ^= st >> 17; st ^= st << 5;
        buf[i] = (uint8_t)(st >> 24);
    }
    /* Lengths hitting every loop-shape combination: wide-only, wide+narrow,
     * narrow-only, multi outer iterations (max_iters re-entry). */
    static const size_t lens[] = { 16, 32, 48, 64, 96, 112, 128, 256, 512,
                                   1024, 4096, 5552, 5568, 11104, 11136,
                                   16704, 65536, 0 };
    for (int i = 0; lens[i]; i++) {
        size_t len = lens[i];
        uint32_t got = vec_adler(1u, buf, len);
        uint32_t want = ref_adler(1u, buf, len);
        if (got != want) return 1;
        got = vec_adler(0xBEEF0001u | 0x0001u, buf + 64, len);
        want = ref_adler(0xBEEF0001u | 0x0001u, buf + 64, len);
        if (got != want) return 2;
    }
    return 0;
}
