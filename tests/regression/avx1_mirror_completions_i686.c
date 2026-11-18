/* i686 (-m32 -mavx) counterpart of avx1_mirror_completions.c.
 *
 * Covers the 128-bit variable permutes (_mm_permutevar_ps / _mm_permutevar_pd,
 * lowered through VpermilvarPs128 / VpermilvarPd128 on the i686 backend) and
 * the 256-bit addsub_pd / dp_ps forms. Expected values follow the Intel
 * definitions, the same as the x86-64 fixture; GCC 16.2 must agree.
 *
 * Freestanding (no libc): the host has no 32-bit libc. The exit status is a
 * bitmask with one bit per check, so a mismatch names the failing check.
 *
 * Vectors are copied out with the _mm*_storeu_si* intrinsics (may_alias
 * stores) before the lanes are read as unsigned int. Reading a vector object
 * through an unsigned int pointer is a strict-aliasing violation that lets the
 * compiler delete the vector store.
 */
#include <immintrin.h>

static int bad; /* bitmask: one bit per failed check */

static void check128(__m128i got, const unsigned int *want, int bit)
{
    unsigned int g[4];
    _mm_storeu_si128((__m128i *)g, got);
    for (int i = 0; i < 4; i++) {
        if (g[i] != want[i]) {
            bad |= bit;
        }
    }
}

static void check256(__m256i got, const unsigned int *want, int bit)
{
    unsigned int g[8];
    _mm256_storeu_si256((__m256i *)g, got);
    for (int i = 0; i < 8; i++) {
        if (g[i] != want[i]) {
            bad |= bit;
        }
    }
}

__attribute__((noinline)) static __m128 permutevar_ps(__m128 v, __m128i idx)
{
    return _mm_permutevar_ps(v, idx);
}

__attribute__((noinline)) static __m128d permutevar_pd(__m128d v, __m128i idx)
{
    return _mm_permutevar_pd(v, idx);
}

__attribute__((noinline)) static __m256d addsub_pd(__m256d a, __m256d b)
{
    return _mm256_addsub_pd(a, b);
}

__attribute__((noinline)) static __m256 dp_ps(__m256 a, __m256 b)
{
    return _mm256_dp_ps(a, b, 0xF1);
}

__attribute__((noinline)) static void permutes(void)
{
    /* permutevar_ps: v = [1 2 3 4], idx lanes = [3 2 1 0] -> [4 3 2 1]. */
    {
        __m128 v = _mm_set_ps(4.0f, 3.0f, 2.0f, 1.0f);
        __m128i idx = _mm_set_epi32(0, 1, 2, 3);
        __m128 got = permutevar_ps(v, idx);
        const unsigned int want[4] = {
            0x40800000u, 0x40400000u, 0x40000000u, 0x3f800000u /* 4 3 2 1 */
        };
        check128(_mm_castps_si128(got), want, 1);
    }

    /* permutevar_ps with idx bits above the low two set: only idx & 3 counts.
     * _mm_set_epi32 fills lanes top-down, so lane idx = [0xe 0x1 0x4 0x7];
     * masked to [2 1 0 3] this selects v = [1 2 3 4] -> [3 2 1 4]. */
    {
        __m128 v = _mm_set_ps(4.0f, 3.0f, 2.0f, 1.0f);
        __m128i idx = _mm_set_epi32(0x7, 0x4, 0x1, 0xe);
        __m128 got = permutevar_ps(v, idx);
        const unsigned int want[4] = {
            0x40400000u, 0x40000000u, 0x3f800000u, 0x40800000u /* 3 2 1 4 */
        };
        check128(_mm_castps_si128(got), want, 2);
    }

    /* permutevar_pd: v = [1 2], idx qwords = [2 0]; bit 1 selects element.
     * Lane 0 takes v[1] = 2.0, lane 1 takes v[0] = 1.0. */
    {
        __m128d v = _mm_set_pd(2.0, 1.0);
        __m128i idx = _mm_set_epi64x(0, 2);
        __m128d got = permutevar_pd(v, idx);
        const unsigned int want[4] = {
            0x00000000u, 0x40000000u, /* 2.0 */
            0x00000000u, 0x3ff00000u  /* 1.0 */
        };
        check128(_mm_castpd_si128(got), want, 4);
    }
}

__attribute__((noinline)) static void wide(void)
{
    /* addsub_pd: a = [5 6 7 8], b = [1 2 3 4] -> [4 8 4 12]. */
    {
        __m256d a = _mm256_set_pd(8.0, 7.0, 6.0, 5.0);
        __m256d b = _mm256_set_pd(4.0, 3.0, 2.0, 1.0);
        __m256d got = addsub_pd(a, b);
        const unsigned int want[8] = {
            0x00000000u, 0x40100000u, /* 4.0  */
            0x00000000u, 0x40200000u, /* 8.0  */
            0x00000000u, 0x40100000u, /* 4.0  */
            0x00000000u, 0x40280000u  /* 12.0 */
        };
        check256(_mm256_castpd_si256(got), want, 8);
    }

    /* dp_ps, imm 0xF1: lanes 0 and 4 receive 60; all others are +0.0. */
    {
        __m256 a = _mm256_set_ps(8, 7, 6, 5, 4, 3, 2, 1);
        __m256 b = _mm256_set_ps(1, 2, 3, 4, 5, 6, 7, 8);
        __m256 got = dp_ps(a, b);
        const unsigned int want[8] = {
            0x42700000u, 0, 0, 0, 0x42700000u, 0, 0, 0 /* 60.0f */
        };
        check256(_mm256_castps_si256(got), want, 16);
    }
}

/* Entry stub: realign %esp before entering C. The kernel enters _start with a
 * 16-byte-aligned stack, but a compiler-generated _start may push registers
 * first, and the aligned __m128/__m256 spills then fault. */
__asm__(".text\n.globl _start\n.type _start, @function\n_start:\n"
        "\tandl $-16, %esp\n\tcall c_start\n\tud2\n");

__attribute__((noreturn)) void c_start(void)
{
    permutes();
    wide();
    int status = bad;
    __asm__ volatile("int $0x80" : : "a"(1), "b"(status) : "memory");
    __builtin_unreachable();
}
