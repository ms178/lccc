// FLAGS: -mavx512f -mavx512bw -mavx2
/* Element shifts by a count at and past the lane, as a constant and as a
 * value of the running program.
 *
 * The instructions saturate: a count past the lane clears it, or fills it
 * with the sign. Three things were wrong at the edges:
 *   - `_mm_srai_pi16`/`_mm_srai_pi32` took a negative count for 0 (it is a
 *     count past the lane: the hardware reads it as unsigned);
 *   - the 512-bit shifts refused a count that is no constant, which GCC
 *     takes (`vpslld %xmm1, %zmm0, %zmm0`), and refused `-1` as a constant;
 *   - a constant past 255 went to the assembler as it stood
 *     (`vpsrad $256`) and was refused there. */
#include <immintrin.h>
#include <stdio.h>
#include <string.h>

static void row512(const char *what, long long count, __m512i v)
{
    unsigned long long u[8];
    memcpy(u, &v, sizeof u);
    printf("%s %lld: %016llx %016llx\n", what, count, u[0], u[7]);
}

static void row256(const char *what, int count, __m256i v)
{
    unsigned long long u[4];
    memcpy(u, &v, sizeof u);
    printf("%s %d: %016llx %016llx\n", what, count, u[0], u[3]);
}

static void row128(const char *what, int count, __m128i v)
{
    unsigned long long u[2];
    memcpy(u, &v, sizeof u);
    printf("%s %d: %016llx %016llx\n", what, count, u[0], u[1]);
}

__attribute__((noinline)) static void by_runtime_count(__m512i a, unsigned count)
{
    row512("slli16 n", count, _mm512_slli_epi16(a, count));
    row512("srli16 n", count, _mm512_srli_epi16(a, count));
    row512("srai16 n", count, _mm512_srai_epi16(a, count));
    row512("slli32 n", count, _mm512_slli_epi32(a, count));
    row512("srli32 n", count, _mm512_srli_epi32(a, count));
    row512("srai32 n", count, _mm512_srai_epi32(a, count));
    row512("slli64 n", count, _mm512_slli_epi64(a, count));
    row512("srli64 n", count, _mm512_srli_epi64(a, count));
    row512("srai64 n", count, _mm512_srai_epi64(a, count));
}

__attribute__((noinline)) static void narrower_by_runtime_count(__m256i a, __m128i b, int count)
{
    row256("256 slli32 n", count, _mm256_slli_epi32(a, count));
    row256("256 srai16 n", count, _mm256_srai_epi16(a, count));
    row256("256 srli64 n", count, _mm256_srli_epi64(a, count));
    row128("128 slli16 n", count, _mm_slli_epi16(b, count));
    row128("128 srai32 n", count, _mm_srai_epi32(b, count));
    row128("128 srli64 n", count, _mm_srli_epi64(b, count));
}

__attribute__((noinline)) static void mmx_by_runtime_count(__m64 a, __m64 b, int count)
{
    printf("srai_pi16 %d: %016llx\n", count, (unsigned long long)_mm_cvtm64_si64(_mm_srai_pi16(a, count)));
    printf("srai_pi32 %d: %016llx\n", count, (unsigned long long)_mm_cvtm64_si64(_mm_srai_pi32(b, count)));
    printf("slli_pi16 %d: %016llx\n", count, (unsigned long long)_mm_cvtm64_si64(_mm_slli_pi16(a, count)));
    printf("srli_pi32 %d: %016llx\n", count, (unsigned long long)_mm_cvtm64_si64(_mm_srli_pi32(b, count)));
}

int main(void)
{
    volatile long long seed = 0x8000000180018081LL;
    __m512i a = _mm512_set1_epi64(seed);
    __m256i b = _mm256_set1_epi64x(seed);
    __m128i c = _mm_set1_epi64x(seed);
    __m64 m16 = _mm_set_pi16(-32768, 0x1234, -2, 1);
    __m64 m32 = _mm_set_pi32(-2147483647 - 1, 0x12345678);
    static const unsigned counts[] = { 0, 1, 7, 15, 16, 17, 31, 32, 33, 63, 64, 65, 255, 256, 4096,
                                       0x7fffffffu, 0x80000000u, 0xffffffffu };

    /* Constants: inside the lane, at it, past the byte the immediate is,
     * negative, and written as unary expressions. */
    row512("slli32", 31, _mm512_slli_epi32(a, 31));
    row512("slli32", 32, _mm512_slli_epi32(a, 32));
    row512("slli32", 256, _mm512_slli_epi32(a, 256));
    row512("srai32", 255, _mm512_srai_epi32(a, 255));
    row512("srai32", 256, _mm512_srai_epi32(a, 256));
    row512("srai32", -1, _mm512_srai_epi32(a, -1));
    row512("srai32", ~0, _mm512_srai_epi32(a, ~0));
    row512("srli64", 300, _mm512_srli_epi64(a, 300));
    row512("srli64", 63, _mm512_srli_epi64(a, 63));
    row512("slli16", 4096, _mm512_slli_epi16(a, 4096));
    row512("slli16", 15, _mm512_slli_epi16(a, 15));
    row512("srai64", 65, _mm512_srai_epi64(a, 65));
    row512("srai64", 3, _mm512_srai_epi64(a, +3));
    row512("srai16", -(1), _mm512_srai_epi16(a, -(1)));
    row512("srli16", !0, _mm512_srli_epi16(a, !0));

    for (unsigned i = 0; i < sizeof counts / sizeof counts[0]; i++) {
        by_runtime_count(a, counts[i]);
        narrower_by_runtime_count(b, c, (int)counts[i]);
        mmx_by_runtime_count(m16, m32, (int)counts[i]);
    }
    return 0;
}
