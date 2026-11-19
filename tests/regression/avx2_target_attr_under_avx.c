/* AVX2 names used from target("avx2") functions in a TU compiled with -mavx.
 *
 * GCC 16.2 declares _mm256_blend_epi32 and _mm256_shuffle_epi32 unconditionally
 * and enforces the ISA at the call site, so a function carrying
 * __attribute__((target("avx2"))) may call them even when the translation unit
 * is compiled with only -mavx.  lccc hid them behind `#ifdef __AVX2__`, so the
 * same calls were implicit declarations.  Both are ISA-gated at codegen, so
 * declaring them unconditionally keeps a plain -mavx caller rejected.
 *
 * Each result is checked lane-by-lane against a scalar reference, so the binary
 * is self-checking.  The AVX2 code only runs when the CPU reports AVX2.
 * Pure-C AVX2 wrappers (for example _mm256_sll_epi16) are deliberately not in
 * this fixture: they remain behind __AVX2__ because they have no codegen gate.
 * Built with -O2 -mavx (see avx2_target_attr_under_avx.flags). */
#include <immintrin.h>
#include <stdio.h>
#include <string.h>

__attribute__((target("avx2"))) static __m256i blend_then_add_16(__m256i a, __m256i b)
{
    return _mm256_add_epi16(_mm256_blend_epi32(a, b, 0x0F), b);
}

__attribute__((target("avx2"))) static __m256i shuffle_dwords(__m256i a)
{
    return _mm256_shuffle_epi32(a, 0x1B);
}

static int check_u16(const char *what, const unsigned short *got, const unsigned short *want, int n)
{
    int bad = 0;
    printf("%s:", what);
    for (int i = 0; i < n; i++) {
        printf(" %u", (unsigned)got[i]);
        if (got[i] != want[i])
            bad++;
    }
    printf(bad ? "  MISMATCH\n" : "  ok\n");
    return bad;
}

static int check_u32(const char *what, const unsigned *got, const unsigned *want, int n)
{
    int bad = 0;
    printf("%s:", what);
    for (int i = 0; i < n; i++) {
        printf(" %u", got[i]);
        if (got[i] != want[i])
            bad++;
    }
    printf(bad ? "  MISMATCH\n" : "  ok\n");
    return bad;
}

int main(void)
{
    if (!__builtin_cpu_supports("avx2")) {
        printf("skip: CPU lacks AVX2\n");
        return 0;
    }

    unsigned short a16[16], b16[16];
    unsigned a32[8], b32[8];
    for (int i = 0; i < 16; i++) {
        a16[i] = (unsigned short)(0x1000u * (unsigned)(i + 1) + 0x0F0Fu);
        b16[i] = (unsigned short)(0x0303u * (unsigned)(i + 3) ^ 0x8001u);
    }
    for (int i = 0; i < 8; i++) {
        a32[i] = 0x01010101u * (unsigned)(i + 1);
        b32[i] = 0xA0000000u + (unsigned)i;
    }

    int bad = 0;
    __m256i va16 = _mm256_loadu_si256((const __m256i *)a16);
    __m256i vb16 = _mm256_loadu_si256((const __m256i *)b16);
    __m256i va32 = _mm256_loadu_si256((const __m256i *)a32);
    __m256i vb32 = _mm256_loadu_si256((const __m256i *)b32);

    /* _mm256_blend_epi32 with 0x0F takes dwords 0..3 from b, 4..7 from a;
     * the add then wraps per 16-bit lane. */
    {
        unsigned short got[16], want[16];
        _mm256_storeu_si256((__m256i *)got, blend_then_add_16(va16, vb16));
        unsigned short blended[16];
        for (int j = 0; j < 8; j++) {
            int from_b = (0x0F >> j) & 1;
            memcpy(blended + 2 * j, from_b ? b16 + 2 * j : a16 + 2 * j, 4);
        }
        for (int i = 0; i < 16; i++)
            want[i] = (unsigned short)(blended[i] + b16[i]);
        bad += check_u16("blend_add_epi16", got, want, 16);
    }

    /* _mm256_shuffle_epi32 with 0x1B reverses the four dwords of each 128-bit lane. */
    {
        unsigned got[8], want[8];
        _mm256_storeu_si256((__m256i *)got, shuffle_dwords(va32));
        for (int lane = 0; lane < 2; lane++)
            for (int j = 0; j < 4; j++)
                want[lane * 4 + j] = a32[lane * 4 + (3 - j)];
        bad += check_u32("shuffle_epi32", got, want, 8);
    }

    printf("failures %d\n", bad);
    return bad ? 1 : 0;
}
