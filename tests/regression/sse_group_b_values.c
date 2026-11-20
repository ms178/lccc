/* SSE2 / SSE4.1 intrinsics that were missing from lccc's headers (GCC 16.2
 * accepts them under -mavx):
 *
 *   SSE2   _mm_sll_epi32, _mm_sll_epi64, _mm_srl_epi32, _mm_srl_epi64,
 *          _mm_sra_epi16, _mm_sra_epi32   (count from the low 64 bits of XMM)
 *   SSE2   _mm_bslli_si128, _mm_bsrli_si128
 *   SSE4.1 _mm_cvtepi8_epi64, _mm_cvtepu8_epi64, _mm_cvtepi16_epi64,
 *          _mm_cvtepu16_epi64
 *   SSE4.1 _mm_minpos_epu16, _mm_mpsadbw_epu8
 *
 * Every result is printed as raw bits. Each count is tested at 0, one below
 * the width, the width, one above it, and at a count with high bits set (a
 * huge unsigned value must act as "above the width", not wrap). The binary
 * must print the same bytes as GCC 16.2. */
#include <immintrin.h>
#include <stdio.h>
#include <string.h>

static void dump_si(const char *tag, __m128i v)
{
    unsigned long long r[2];
    _mm_storeu_si128((__m128i *)r, v);
    printf("%s %016llx %016llx\n", tag, r[0], r[1]);
}

static __m128i count_of(unsigned long long c)
{
    return _mm_set_epi64x(0x1234567890abcdefLL, (long long)c);
}

static void shift_cases(void)
{
    __m128i a32 = _mm_set_epi32(0x80000001, 0x7fffffff, -2, 0x12345678);
    __m128i a64 = _mm_set_epi64x(-0x123456789LL, 0x0fedcba987654321LL);
    __m128i a16 = _mm_set_epi16(-32768, 32767, -3, 0x0102, -1, 0x7000, 0x8001, 0x00ff);

    static const unsigned long long counts[] = {
        0, 1, 15, 16, 31, 32, 63, 64, 65, 0x100000000ULL, 0xffffffffffffffffULL,
    };
    for (unsigned i = 0; i < sizeof counts / sizeof counts[0]; i++) {
        unsigned long long c = counts[i];
        char tag[64];
        snprintf(tag, sizeof tag, "sll32_%llx", c);
        dump_si(tag, _mm_sll_epi32(a32, count_of(c)));
        snprintf(tag, sizeof tag, "sll64_%llx", c);
        dump_si(tag, _mm_sll_epi64(a64, count_of(c)));
        snprintf(tag, sizeof tag, "srl32_%llx", c);
        dump_si(tag, _mm_srl_epi32(a32, count_of(c)));
        snprintf(tag, sizeof tag, "srl64_%llx", c);
        dump_si(tag, _mm_srl_epi64(a64, count_of(c)));
        snprintf(tag, sizeof tag, "sra16_%llx", c);
        dump_si(tag, _mm_sra_epi16(a16, count_of(c)));
        snprintf(tag, sizeof tag, "sra32_%llx", c);
        dump_si(tag, _mm_sra_epi32(a32, count_of(c)));
    }

    /* The count is read from the low 64 bits only; the high 64 bits of the
     * count register must not matter. */
    dump_si("sll32_hi_ignored", _mm_sll_epi32(a32, _mm_set_epi64x(0x7fffffffffffffffLL, 2LL)));
    dump_si("sra16_hi_ignored", _mm_sra_epi16(a16, _mm_set_epi64x(0LL, 3LL)));
}

static void byte_shift_cases(void)
{
    __m128i v = _mm_set_epi8(16, 15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1);
    dump_si("bslli_0", _mm_bslli_si128(v, 0));
    dump_si("bslli_3", _mm_bslli_si128(v, 3));
    dump_si("bslli_16", _mm_bslli_si128(v, 16));
    dump_si("bsrli_1", _mm_bsrli_si128(v, 1));
    dump_si("bsrli_9", _mm_bsrli_si128(v, 9));
    dump_si("bsrli_16", _mm_bsrli_si128(v, 16));
}

static void extend_cases(void)
{
    /* Negative bytes and shorts: sign extension must differ from zero
     * extension. Elements beyond the first two must not leak in. */
    __m128i v = _mm_set_epi8(0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc,
                             0xdd, 0xee, 0xff, 0x80, 0x7f, 0x01, 0xfe, 0x00);
    dump_si("cvtepi8_epi64", _mm_cvtepi8_epi64(v));
    dump_si("cvtepu8_epi64", _mm_cvtepu8_epi64(v));
    __m128i w = _mm_set_epi16(0x7000, 0x0001, 0x8000, 0xffff, 0x1234, 0x9abc, 0x0000, 0xfffe);
    dump_si("cvtepi16_epi64", _mm_cvtepi16_epi64(w));
    dump_si("cvtepu16_epi64", _mm_cvtepu16_epi64(w));
}

static void minpos_cases(void)
{
    /* Tie for the minimum: the first occurrence wins (index 2, not 5). */
    dump_si("minpos_tie", _mm_minpos_epu16(_mm_set_epi16(9, 3, 7, 0x8000, 1, 0xffff, 1, 5)));
    /* Unsigned compare: 0x8000 is larger than 0x7fff, not smaller. */
    dump_si("minpos_unsigned", _mm_minpos_epu16(_mm_set_epi16(0x8000, 0x7fff, 0xffff, 0x8001, 0x7ffe, 0x9000, 0x7fff, 0xfffe)));
    dump_si("minpos_first", _mm_minpos_epu16(_mm_set_epi16(1, 1, 1, 1, 1, 1, 1, 0)));
    dump_si("minpos_zero", _mm_minpos_epu16(_mm_setzero_si128()));
}

static void mpsad_cases(void)
{
    __m128i a = _mm_set_epi8(0x10, 0x20, 0x30, 0x40, 0x50, 0x60, 0x70, 0x80,
                             0x90, 0xa0, 0xb0, 0xc0, 0xd0, 0xe0, 0xf0, 0x05);
    __m128i b = _mm_set_epi8(0xff, 0x01, 0x7f, 0x00, 0x33, 0xcc, 0x99, 0x66,
                             0x11, 0x22, 0x44, 0x88, 0xef, 0xfe, 0x00, 0x7e);
    dump_si("mpsad_0", _mm_mpsadbw_epu8(a, b, 0));
    dump_si("mpsad_1", _mm_mpsadbw_epu8(a, b, 1));
    dump_si("mpsad_2", _mm_mpsadbw_epu8(a, b, 2));
    dump_si("mpsad_3", _mm_mpsadbw_epu8(a, b, 3));
    dump_si("mpsad_4", _mm_mpsadbw_epu8(a, b, 4));
    dump_si("mpsad_5", _mm_mpsadbw_epu8(a, b, 5));
    dump_si("mpsad_6", _mm_mpsadbw_epu8(a, b, 6));
    dump_si("mpsad_7", _mm_mpsadbw_epu8(a, b, 7));
    /* Maximum: 4 * 255 per result, with no 16-bit overflow. */
    dump_si("mpsad_max", _mm_mpsadbw_epu8(_mm_set1_epi8((char)0xff), _mm_setzero_si128(), 0));
}

int main(void)
{
    shift_cases();
    byte_shift_cases();
    extend_cases();
    minpos_cases();
    mpsad_cases();
    printf("done\n");
    return 0;
}
