/* BUG-003: inlined Adler leftover (len & 1 / len & 2) must do
 *   adler += byte; sum2 += adler;
 * not sum2 += byte.
 *
 * fold_accumulator_alu_store used to rewrite
 *   movl %regd, %eax; addl slot, %eax; movl %eax, dest
 * into addl slot, %regd and leave later uses of %eax reading the pre-add
 * copy (the byte). Triggered when copy_tail is inlined into a large
 * SIMD function (zlib-ng adler32_ssse3). */
#include <stdint.h>
#include <stdio.h>
#include <emmintrin.h>

#define BASE 65521U
#define MIN(a, b) ((a) < (b) ? (a) : (b))
#define ALIGN_DOWN(x, a) ((x) & ~((size_t)(a) - 1))
#define ADLER32_SWAR_MAX_BYTES (23 * 8)
#define ADLER32_SWAR_EVEN_MASK 0x00FF00FF00FF00FFULL
#define ADLER32_SWAR_HSUM 0x1000100010001ULL
#define ADLER_DO1(sum1, sum2, buf, i) \
    {                                 \
        (sum1) += buf[(i)];           \
        (sum2) += (sum1);             \
    }
#define ADLER_DO2(sum1, sum2, buf, i)      \
    {                                      \
        ADLER_DO1(sum1, sum2, buf, i);     \
        ADLER_DO1(sum1, sum2, buf, i + 1); \
    }
#define ADLER_DO4(sum1, sum2, buf, i)      \
    {                                      \
        ADLER_DO2(sum1, sum2, buf, i);     \
        ADLER_DO2(sum1, sum2, buf, i + 2); \
    }

__attribute__((always_inline)) static void
adler32_swar(uint32_t *adler, const uint8_t *buf, size_t len, uint32_t *sum2) {
    uint64_t sum_even = 0, sum_odd = 0, prefix_even = 0, prefix_odd = 0;
    *sum2 += *adler * (uint32_t)len;
    const uint64_t *src64 = (const uint64_t *)buf;
    if (len >= 8) {
        uint64_t v = *src64;
        prefix_even += sum_even;
        prefix_odd += sum_odd;
        sum_even += v & ADLER32_SWAR_EVEN_MASK;
        sum_odd += (v >> 8) & ADLER32_SWAR_EVEN_MASK;
    }
    *adler += (uint32_t)(((sum_even + sum_odd) * ADLER32_SWAR_HSUM) >> 48);
    uint64_t pe_lo = prefix_even & 0xFFFF0000FFFFULL;
    uint64_t pe_hi = (prefix_even >> 16) & 0xFFFF0000FFFFULL;
    uint64_t po_lo = prefix_odd & 0xFFFF0000FFFFULL;
    uint64_t po_hi = (prefix_odd >> 16) & 0xFFFF0000FFFFULL;
    *sum2 += (uint32_t)(((pe_lo + po_lo + pe_hi + po_hi) * 0x800000008ULL) >> 32);
    *sum2 += 2 * (uint32_t)((sum_even * 0x4000300020001ULL) >> 48)
           + (uint32_t)((sum_odd * ADLER32_SWAR_HSUM) >> 48)
           + 2 * (uint32_t)((sum_odd * 0x3000200010000ULL) >> 48);
}

__attribute__((always_inline)) static uint32_t
adler32_copy_tail(uint32_t adler, const uint8_t *buf, size_t len, uint32_t sum2) {
    if (len) {
        while (len >= 8 && ((uintptr_t)buf & 7) == 0) {
            size_t chunk = MIN(ALIGN_DOWN(len, (size_t)8), (size_t)ADLER32_SWAR_MAX_BYTES);
            adler32_swar(&adler, buf, chunk, &sum2);
            buf += chunk;
            len -= chunk;
        }
        while (len >= 4) {
            len -= 4;
            ADLER_DO4(adler, sum2, buf, 0);
            buf += 4;
        }
        if (len & 2) {
            ADLER_DO2(adler, sum2, buf, 0);
            buf += 2;
        }
        if (len & 1) {
            ADLER_DO1(adler, sum2, buf, 0);
        }
    }
    adler %= BASE;
    sum2 %= BASE;
    return adler | (sum2 << 16);
}

/* Same shape as zlib-ng adler32_ssse3: early-return tail + function-scope XMM. */
__attribute__((noinline)) uint32_t like_ssse3(uint32_t adler, const uint8_t *buf, size_t len) {
    uint32_t sum2 = (adler >> 16) & 0xffff;
    adler &= 0xffff;
    if (len == 1)
        return adler32_copy_tail(adler, buf, 1, sum2);
    if (len < 16)
        return adler32_copy_tail(adler, buf, len, sum2);

    __m128i vbuf, vs1, vs2, vs3, vs1_0, vs2_0, v_sad_sum1, v_short_sum2, vsum2;
    __m128i vbuf_0, v_sad_sum2, v_short_sum2_0, vsum2_0;
    const __m128i zero = _mm_setzero_si128();
    vs1 = _mm_cvtsi32_si128((int)adler);
    vs2 = _mm_cvtsi32_si128((int)sum2);
    vbuf = _mm_loadu_si128((const __m128i *)buf);
    v_sad_sum1 = _mm_sad_epu8(vbuf, zero);
    vs1 = _mm_add_epi32(v_sad_sum1, vs1);
    (void)vs3;
    (void)vs1_0;
    (void)vs2_0;
    (void)v_short_sum2;
    (void)vsum2;
    (void)vbuf_0;
    (void)v_sad_sum2;
    (void)v_short_sum2_0;
    (void)vsum2_0;
    return (uint32_t)_mm_cvtsi128_si32(vs1);
}

static uint32_t ref32(uint32_t a, const uint8_t *b, size_t n) {
    uint32_t s1 = a & 0xffff, s2 = (a >> 16) & 0xffff;
    for (size_t i = 0; i < n; i++) {
        s1 += b[i];
        s2 += s1;
        if (s1 >= 65521)
            s1 -= 65521;
        if (s2 >= 65521)
            s2 -= 65521;
    }
    return s1 | (s2 << 16);
}

int main(void) {
    const uint8_t *s = (const uint8_t *)"123456789";
    int bad = 0;
    for (int n = 1; n <= 15; n++) {
        uint32_t g = like_ssse3(1, s, (size_t)n);
        uint32_t r = ref32(1, s, (size_t)n);
        if (g != r) {
            printf("FAIL n=%d got=%08x ref=%08x\n", n, g, r);
            bad = 1;
        }
    }
    if (!bad)
        printf("OK adler_inline_tail\n");
    return bad;
}
