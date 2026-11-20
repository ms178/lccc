/* Regression: the SSE2 packed shifts by a scalar count taken from the low 64
 * bits of an XMM register (_mm_sll/srl/sra_epi16/32/64 forms: PSLLW/PSLLD/PSLLQ,
 * PSRLW/PSRLD/PSRLQ, PSRAW/PSRAD with an XMM count).
 *
 * Every result is printed as its four 32-bit lanes. The output must match GCC
 * 16.2 byte for byte. The counts cover 0, 1, the element width minus one, the
 * width itself, the 64-bit maximum, and two values whose low quadword is past
 * every width but whose high quadword is nonzero. Only the LOW 64 bits of the
 * count register are architectural: a count of 2^32+3 must saturate, not
 * shift by 3. Logical shifts give zero past the width; arithmetic shifts fill
 * with the sign bit. The inputs are read through volatile storage, so the
 * compiler cannot fold the shifts away. */
#include <emmintrin.h>
#include <stdio.h>
#include <string.h>

static volatile unsigned int src_words[4] = {0x80000001u, 0xFFFFFFFFu, 0x7FFFFFFFu, 0x00000002u};
static volatile unsigned int neg_words[4] = {0xFFFF8000u, 0x80000000u, 0xFFFFFFFEu, 0x12348765u};

static const unsigned long long counts[] = {
    0ULL, 1ULL, 15ULL, 16ULL, 31ULL, 32ULL, 63ULL, 64ULL, 200ULL,
    0xFFFFFFFFFFFFFFFFULL, 0x100000003ULL, 0x8000000000000000ULL,
};

static void show(const char *tag, __m128i v)
{
    unsigned int w[4];
    _mm_storeu_si128((__m128i *)w, v);
    printf("%-18s %08x %08x %08x %08x\n", tag, w[0], w[1], w[2], w[3]);
}

/* High quadword is nonzero on purpose: it must be ignored by every form. */
static __m128i count_vec(unsigned long long lo)
{
    return _mm_set_epi64x((long long)0x0123456789abcdefLL, (long long)lo);
}

static void run_all(const char *which, __m128i a, unsigned long long c)
{
    char tag[32];
    __m128i k = count_vec(c);
    snprintf(tag, sizeof tag, "%s sll16 %llu", which, c);
    show(tag, _mm_sll_epi16(a, k));
    snprintf(tag, sizeof tag, "%s sll32 %llu", which, c);
    show(tag, _mm_sll_epi32(a, k));
    snprintf(tag, sizeof tag, "%s sll64 %llu", which, c);
    show(tag, _mm_sll_epi64(a, k));
    snprintf(tag, sizeof tag, "%s srl16 %llu", which, c);
    show(tag, _mm_srl_epi16(a, k));
    snprintf(tag, sizeof tag, "%s srl32 %llu", which, c);
    show(tag, _mm_srl_epi32(a, k));
    snprintf(tag, sizeof tag, "%s srl64 %llu", which, c);
    show(tag, _mm_srl_epi64(a, k));
    snprintf(tag, sizeof tag, "%s sra16 %llu", which, c);
    show(tag, _mm_sra_epi16(a, k));
    snprintf(tag, sizeof tag, "%s sra32 %llu", which, c);
    show(tag, _mm_sra_epi32(a, k));
}

int main(void)
{
    __m128i pos = _mm_loadu_si128((const __m128i *)src_words);
    __m128i neg = _mm_loadu_si128((const __m128i *)neg_words);
    size_t n = sizeof counts / sizeof counts[0];
    for (size_t i = 0; i < n; i++)
        run_all("pos", pos, counts[i]);
    for (size_t i = 0; i < n; i++)
        run_all("neg", neg, counts[i]);
    return 0;
}
