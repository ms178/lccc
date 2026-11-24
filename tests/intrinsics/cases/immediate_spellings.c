// FLAGS: -msse4.1 -mavx2
/* An immediate is an integer constant expression, however it is spelled.
 *
 * A literal and a binary operation of literals were folded where the call
 * is lowered; a unary expression (`-(-1)`, `~~2`, `!0`) was left to the
 * optimizer. Without optimization the emitter was then given a value where
 * it encodes a constant, and encoded 0: `_mm_extract_epi8(v, -(-1))`
 * returned byte 0. Every row below names the same element or control word
 * in another spelling, so every row of a group prints the same value.
 *
 * The last group is what GCC takes although it does not fit the byte: for
 * the 128-bit integer shuffles and the floating-point shuffles GCC accepts
 * any `int` and encodes its low eight bits. */
#include <immintrin.h>
#include <stdio.h>
#include <string.h>

enum { LANE_TWO = 2, CONTROL = 0x1b };

static void row(const char *what, __m128i v)
{
    unsigned u[4];
    memcpy(u, &v, sizeof u);
    printf("%-22s %08x %08x %08x %08x\n", what, u[0], u[1], u[2], u[3]);
}

static void row256(const char *what, __m256i v)
{
    unsigned long long u[4];
    memcpy(u, &v, sizeof u);
    printf("%-22s %016llx %016llx %016llx %016llx\n", what, u[0], u[1], u[2], u[3]);
}

int main(void)
{
    volatile unsigned seed = 0x03020100u;
    __m128i v = _mm_set_epi32((int)(seed + 0x0c0c0c0cu), (int)(seed + 0x08080808u),
                              (int)(seed + 0x04040404u), (int)seed);
    __m128i w = _mm_set1_epi32((int)(seed * 3u + 0x11111111u));
    __m256i y = _mm256_set_epi64x((long long)seed * 7, (long long)seed * 5, (long long)seed * 3,
                                  (long long)seed);
    __m128 f = _mm_castsi128_ps(v), g = _mm_castsi128_ps(w);

    /* element 2 of sixteen bytes, eight words, four doublewords */
    printf("extract8  %d %d %d %d %d %d %d\n", _mm_extract_epi8(v, 2), _mm_extract_epi8(v, -(-2)),
           _mm_extract_epi8(v, ~~2), _mm_extract_epi8(v, LANE_TWO), _mm_extract_epi8(v, 1 + 1),
           _mm_extract_epi8(v, (unsigned char)2), _mm_extract_epi8(v, 1 ? 2 : 3));
    printf("extract16 %d %d %d %d\n", _mm_extract_epi16(v, 2), _mm_extract_epi16(v, -(-2)),
           _mm_extract_epi16(v, '\2'), _mm_extract_epi16(v, +2));
    printf("extract32 %d %d %d %d\n", _mm_extract_epi32(v, 2), _mm_extract_epi32(v, -(-2)),
           _mm_extract_epi32(v, (int)sizeof(short)), _mm_extract_epi32(v, !0 + !0));
    printf("extract64 %llx %llx %llx\n", (unsigned long long)_mm_extract_epi64(v, 1),
           (unsigned long long)_mm_extract_epi64(v, -(-1)),
           (unsigned long long)_mm_extract_epi64(v, !0));

    row("insert8 2", _mm_insert_epi8(v, 0x5a, 2));
    row("insert8 -(-2)", _mm_insert_epi8(v, 0x5a, -(-2)));
    row("insert16 ~~2", _mm_insert_epi16(v, 0x5a5a, ~~2));
    row("insert16 2", _mm_insert_epi16(v, 0x5a5a, 2));
    row("insert32 2", _mm_insert_epi32(v, 0x5a5a5a5a, 2));
    row("insert32 LANE_TWO", _mm_insert_epi32(v, 0x5a5a5a5a, LANE_TWO));
    row("insert32 -(-2)", _mm_insert_epi32(v, 0x5a5a5a5a, -(-2)));

    row("shuffle 0x1b", _mm_shuffle_epi32(v, 0x1b));
    row("shuffle -(-0x1b)", _mm_shuffle_epi32(v, -(-0x1b)));
    row("shuffle CONTROL", _mm_shuffle_epi32(v, CONTROL));
    row("shuffle ~~0x1b", _mm_shuffle_epi32(v, ~~0x1b));
    row("shuffle _MM_SHUFFLE", _mm_shuffle_epi32(v, _MM_SHUFFLE(0, 1, 2, 3)));
    row("shufflelo -(-0x1b)", _mm_shufflelo_epi16(v, -(-0x1b)));
    row("shufflelo 0x1b", _mm_shufflelo_epi16(v, 0x1b));
    row("shufflehi -(-0x1b)", _mm_shufflehi_epi16(v, -(-0x1b)));
    row("shufflehi 0x1b", _mm_shufflehi_epi16(v, 0x1b));

    row("blend16 0x5a", _mm_blend_epi16(v, w, 0x5a));
    row("blend16 -(-0x5a)", _mm_blend_epi16(v, w, -(-0x5a)));
    row("alignr 5", _mm_alignr_epi8(v, w, 5));
    row("alignr -(-5)", _mm_alignr_epi8(v, w, -(-5)));
    row("slli_si128 3", _mm_slli_si128(v, 3));
    row("slli_si128 -(-3)", _mm_slli_si128(v, -(-3)));
    row("srli_si128 ~~3", _mm_srli_si128(v, ~~3));
    row("srli_si128 3", _mm_srli_si128(v, 3));
    row("shuffle_ps 0x1b", _mm_castps_si128(_mm_shuffle_ps(f, g, 0x1b)));
    row("shuffle_ps -(-0x1b)", _mm_castps_si128(_mm_shuffle_ps(f, g, -(-0x1b))));
    row("blend_ps 5", _mm_castps_si128(_mm_blend_ps(f, g, 5)));
    row("blend_ps -(-5)", _mm_castps_si128(_mm_blend_ps(f, g, -(-5))));

    row256("permute4x64 0x1b", _mm256_permute4x64_epi64(y, 0x1b));
    row256("permute4x64 -(-0x1b)", _mm256_permute4x64_epi64(y, -(-0x1b)));
    row256("shuffle256 0x1b", _mm256_shuffle_epi32(y, 0x1b));
    row256("shuffle256 ~~0x1b", _mm256_shuffle_epi32(y, ~~0x1b));
    row256("bslli256 3", _mm256_bslli_epi128(y, 3));
    row256("bslli256 -(-3)", _mm256_bslli_epi128(y, -(-3)));
    row256("blend32 0x5a", _mm256_blend_epi32(y, _mm256_set1_epi32(-1), 0x5a));
    row256("blend32 -(-0x5a)", _mm256_blend_epi32(y, _mm256_set1_epi32(-1), -(-0x5a)));
    row("extracti128 1", _mm256_extracti128_si256(y, 1));
    row("extracti128 !0", _mm256_extracti128_si256(y, !0));

    /* Past the byte: GCC encodes the low eight bits. */
    row("shuffle 0x11b", _mm_shuffle_epi32(v, 0x11b));
    row("shuffle -1", _mm_shuffle_epi32(v, -1));
    row("shuffle 256", _mm_shuffle_epi32(v, 256));
    row("shufflelo 0x11b", _mm_shufflelo_epi16(v, 0x11b));
    row("shufflehi -1", _mm_shufflehi_epi16(v, -1));
    row("shuffle_ps 0x11b", _mm_castps_si128(_mm_shuffle_ps(f, g, 0x11b)));
    row("shuffle_ps 256", _mm_castps_si128(_mm_shuffle_ps(f, g, 256)));
    row("shuffle_pd 256+1", _mm_castpd_si128(_mm_shuffle_pd(_mm_castsi128_pd(v), _mm_castsi128_pd(w), 257)));

    /* A floating constant: the parameter is an `int`, so the call converts
     * it, toward zero, as an assignment would. GCC reads 27.9 as 27. It was
     * the selector 0 here (the constant was "no integer"), without a word. */
    row("shuffle 27.9", _mm_shuffle_epi32(v, 27.9));
    row("shuffle 27", _mm_shuffle_epi32(v, 27));
    row("shufflelo 27.5f", _mm_shufflelo_epi16(v, 27.5f));
    row("shufflehi 0x1b.8p0", _mm_shufflehi_epi16(v, 0x1b.8p0));
    row("shuffle -0.5", _mm_shuffle_epi32(v, -0.5));
    row("shuffle 256.5", _mm_shuffle_epi32(v, 256.5));
    row("slli_si128 1.99", _mm_slli_si128(v, 1.99));
    row("srli_si128 3.01L", _mm_srli_si128(v, 3.01L));
    row("alignr 5.5", _mm_alignr_epi8(v, w, 5.5));
    row("blend16 90.9", _mm_blend_epi16(v, w, 90.9));
    row("insert16 3.2", _mm_insert_epi16(v, 0x7777, 3.2));
    row("insert32 2.999", _mm_insert_epi32(v, 0x55667788, 2.999));
    printf("extract16 2.5 %d\n", _mm_extract_epi16(v, 2.5));
    printf("extract32 1.5 %d\n", _mm_extract_epi32(v, 1.5));
    printf("extract8 15.9 %d\n", _mm_extract_epi8(v, 15.9));
    row("shuffle_ps 27.5f", _mm_castps_si128(_mm_shuffle_ps(f, g, 27.5f)));
    row("blend_ps 5.9", _mm_castps_si128(_mm_blend_ps(f, g, 5.9)));
    row("round_ps 1.5", _mm_castps_si128(_mm_round_ps(f, 1.5)));
    row256("permute4x64 27.75", _mm256_permute4x64_epi64(y, 27.75));
    row256("shuffle256 27.25", _mm256_shuffle_epi32(y, 27.25));
    row256("blend32 90.5", _mm256_blend_epi32(y, _mm256_set1_epi32(-1), 90.5));
    row("extracti128 1.9", _mm256_extracti128_si256(y, 1.9));
    /* The spelling of the builtin itself, which Clang refuses with a
     * floating constant ("must be a constant integer"); GCC converts. */
    row("pshufd 3.75", (__m128i)__builtin_ia32_pshufd((__v4si)v, 3.75));
    return 0;
}
