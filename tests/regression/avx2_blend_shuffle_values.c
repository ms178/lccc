/* AVX2 lane-select and wrap-around arithmetic fixture.
 *
 * Pins the values of the AVX2 forms whose definitions changed in this PR:
 *   - _mm256_blend_epi32  (VPBLENDD, 256-bit) and _mm_blend_epi32 (VPBLENDD, 128-bit)
 *   - _mm256_shuffle_epi32 (VPSHUFD, 256-bit)
 *   - _mm256_add_epi16, _mm256_sub_epi16, _mm256_sub_epi8 (now builtins, were scalar loops)
 * Every case uses lane patterns where an off-by-one lane, a sign error, or a
 * dropped carry changes the printed output.  The binary is compared against GCC
 * 16.2 (byte-identical stdout).  Built with -mavx2 only: under -mavx these names
 * are undeclared (see tests/regression/check_avx_intrinsic_isa.py). */
#include <immintrin.h>
#include <stdio.h>

static void dump_i32(const char *tag, __m256i v)
{
    int r[8];
    _mm256_storeu_si256((__m256i *)r, v);
    printf("%s", tag);
    for (int i = 0; i < 8; i++)
        printf(" %d", r[i]);
    printf("\n");
}

static void dump_i32x4(const char *tag, __m128i v)
{
    int r[4];
    _mm_storeu_si128((__m128i *)r, v);
    printf("%s %d %d %d %d\n", tag, r[0], r[1], r[2], r[3]);
}

static void dump_i16(const char *tag, __m256i v)
{
    short r[16];
    _mm256_storeu_si256((__m256i *)r, v);
    printf("%s", tag);
    for (int i = 0; i < 16; i++)
        printf(" %d", r[i]);
    printf("\n");
}

static void dump_i8(const char *tag, __m256i v)
{
    signed char r[32];
    _mm256_storeu_si256((__m256i *)r, v);
    printf("%s", tag);
    for (int i = 0; i < 32; i++)
        printf(" %d", r[i]);
    printf("\n");
}

int main(void)
{
    /* 256-bit blend: every immediate lane pattern selects independently. */
    int A[8] = {1, 2, 3, 4, 5, 6, 7, 8};
    int B[8] = {-11, -12, -13, -14, -15, -16, -17, -18};
    __m256i a = _mm256_loadu_si256((const __m256i *)A);
    __m256i b = _mm256_loadu_si256((const __m256i *)B);
    dump_i32("blend0x00", _mm256_blend_epi32(a, b, 0x00));
    dump_i32("blend0xff", _mm256_blend_epi32(a, b, 0xff));
    dump_i32("blend0x5a", _mm256_blend_epi32(a, b, 0x5a));
    dump_i32("blend0x81", _mm256_blend_epi32(a, b, 0x81));
    dump_i32("blend0xc3", _mm256_blend_epi32(a, b, 0xc3));

    /* 256-bit shuffle: applies within each 128-bit lane. */
    dump_i32("shuf0x1b", _mm256_shuffle_epi32(a, 0x1b));
    dump_i32("shuf0x4e", _mm256_shuffle_epi32(a, 0x4e));
    dump_i32("shuf0x00", _mm256_shuffle_epi32(b, 0x00));

    /* 128-bit blend (VPBLENDD xmm). */
    __m128i x = _mm_set_epi32(4, 3, 2, 1);
    __m128i y = _mm_set_epi32(14, 13, 12, 11);
    dump_i32x4("blend128_9", _mm_blend_epi32(x, y, 0x9));
    dump_i32x4("blend128_6", _mm_blend_epi32(x, y, 0x6));
    dump_i32x4("blend128_f", _mm_blend_epi32(x, y, 0xf));
    dump_i32x4("blend128_0", _mm_blend_epi32(x, y, 0x0));

    /* Wrap-around: 16- and 8-bit subtraction and addition must not saturate
     * and must wrap modulo 2^16 / 2^8.  INT16_MIN - 1 wraps to INT16_MAX;
     * 0x7f - (-1) wraps to -128. */
    short P[16] = {-32768, 32767, 0, -1, 1, 255, 256, -256,
                   100, -100, 7, -7, 1000, -1000, 12345, -12345};
    short Q[16] = {1, 1, -1, -1, 2, 3, 4, 5,
                   -300, 300, 9, -9, -2000, 2000, 1, -1};
    __m256i p = _mm256_loadu_si256((const __m256i *)P);
    __m256i q = _mm256_loadu_si256((const __m256i *)Q);
    dump_i16("sub16", _mm256_sub_epi16(p, q));
    dump_i16("add16", _mm256_add_epi16(p, q));

    signed char C[32], D[32];
    for (int i = 0; i < 32; i++) {
        C[i] = (signed char)(i * 9 - 128);
        D[i] = (signed char)(127 - i * 7);
    }
    __m256i c = _mm256_loadu_si256((const __m256i *)C);
    __m256i d = _mm256_loadu_si256((const __m256i *)D);
    dump_i8("sub8", _mm256_sub_epi8(c, d));

    return 0;
}
