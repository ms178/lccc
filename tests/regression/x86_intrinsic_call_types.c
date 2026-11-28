/* A call of an x86 intrinsic has the type its header gives it, wherever the
 * call stands.
 *
 * lccc lowers some public names itself (`_mm_sll_epi16` is the instruction;
 * no header function exists for it). Forty-eight of those names had no type:
 * a call was an `int`, and its value the address of the 16- or 32-byte
 * result. `__m128i r = _mm_sll_epi16(a, n);` copied the right bytes all the
 * same, which is the one shape every other test of these intrinsics has.
 * Returned from a function the call was a stack address in %xmm0; passed to
 * a function it was a pointer in %rdi; `sizeof` of it was 8.
 *
 * The types of all forty-eight are asserted (nothing is evaluated, so the
 * VNNI and GFNI names need no such processor); the ones an AVX2 machine
 * executes are run in each position, and GCC's output is the reference.
 */
#include <immintrin.h>
#include <stdio.h>

#define SAME(call, type) \
    _Static_assert(__builtin_types_compatible_p(__typeof__(call), type), #call)
#define VNNI(stem)                             \
    SAME(_mm_##stem(a, b, c), __m128i);        \
    SAME(_mm256_##stem(x, y, z), __m256i)

__attribute__((unused)) static void types(__m128i a, __m128i b, __m128i c, __m256i x, __m256i y,
                                          __m256i z)
{
    SAME(_mm_sll_epi16(a, b), __m128i);
    SAME(_mm_sll_epi32(a, b), __m128i);
    SAME(_mm_sll_epi64(a, b), __m128i);
    SAME(_mm_srl_epi16(a, b), __m128i);
    SAME(_mm_srl_epi32(a, b), __m128i);
    SAME(_mm_srl_epi64(a, b), __m128i);
    SAME(_mm_sra_epi16(a, b), __m128i);
    SAME(_mm_sra_epi32(a, b), __m128i);
    SAME(_mm_avg_epu16(a, b), __m128i);
    SAME(_mm_mul_epi32(a, b), __m128i);
    SAME(_mm_gf2p8mul_epi8(a, b), __m128i);
    SAME(_mm_gf2p8affine_epi64_epi8(a, b, 1), __m128i);
    SAME(_mm_gf2p8affineinv_epi64_epi8(a, b, 1), __m128i);
    SAME(_mm256_cmpgt_epi8(x, y), __m256i);
    SAME(_mm256_bslli_epi128(x, 1), __m256i);
    SAME(_mm256_bsrli_epi128(x, 1), __m256i);
    VNNI(dpbusd_avx_epi32);
    VNNI(dpbusds_avx_epi32);
    VNNI(dpwssd_avx_epi32);
    VNNI(dpwssds_avx_epi32);
    VNNI(dpbssd_epi32);
    VNNI(dpbssds_epi32);
    VNNI(dpbsud_epi32);
    VNNI(dpbsuds_epi32);
    VNNI(dpbuud_epi32);
    VNNI(dpbuuds_epi32);
    VNNI(dpwsud_avx_epi32);
    VNNI(dpwsuds_avx_epi32);
    VNNI(dpwusd_avx_epi32);
    VNNI(dpwusds_avx_epi32);
    VNNI(dpwuud_avx_epi32);
    VNNI(dpwuuds_avx_epi32);
    _Static_assert(sizeof(_mm_sll_epi16(a, b)) == 16, "the size of a 128-bit call");
    _Static_assert(sizeof(_mm256_cmpgt_epi8(x, y)) == 32, "the size of a 256-bit call");
}

/* Returned: the value leaves in %xmm0 or %ymm0. */
#define RETURNED(stem)                                                      \
    __attribute__((noinline)) static __m128i returned_##stem(__m128i a, __m128i b) \
    { return _mm_##stem(a, b); }
RETURNED(sll_epi16)
RETURNED(sll_epi32)
RETURNED(sll_epi64)
RETURNED(srl_epi16)
RETURNED(srl_epi32)
RETURNED(srl_epi64)
RETURNED(sra_epi16)
RETURNED(sra_epi32)
RETURNED(avg_epu16)
RETURNED(mul_epi32)
__attribute__((noinline)) static __m256i returned_cmpgt_epi8(__m256i a, __m256i b)
{ return _mm256_cmpgt_epi8(a, b); }
__attribute__((noinline)) static __m256i returned_bslli(__m256i a)
{ return _mm256_bslli_epi128(a, 3); }
__attribute__((noinline)) static __m256i returned_bsrli(__m256i a)
{ return _mm256_bsrli_epi128(a, 5); }

/* Passed on: the callee receives a vector register, not an address. */
__attribute__((noinline)) static unsigned long long fold128(__m128i v)
{
    unsigned long long lanes[2];
    _mm_storeu_si128((__m128i *)lanes, v);
    return lanes[0] * 0x9e3779b97f4a7c15ull ^ lanes[1];
}
__attribute__((noinline)) static unsigned long long fold256(__m256i v)
{
    unsigned long long lanes[4];
    _mm256_storeu_si256((__m256i *)lanes, v);
    return (lanes[0] * 0x9e3779b97f4a7c15ull ^ lanes[1]) * 31 + (lanes[2] ^ lanes[3] * 7);
}

struct box { int tag; __m128i v; };

int main(void)
{
    volatile int which = 1;
    __m128i a = _mm_set_epi16(-32768, 32767, -3, 4, 1000, -1000, 7, -8);
    __m128i b = _mm_set_epi16(5, -6, 7000, -8000, 9, -10, 11, -12);
    __m128i n = _mm_cvtsi32_si128(3);
    __m256i x = _mm256_set_epi8(-128, 127, -1, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14,
                                15, 16, 17, 18, 19, 20, -21, -22, -23, -24, 25, 26, 27, 28);
    __m256i y = _mm256_set1_epi8(9);

    printf("returned sll %llx %llx %llx\n", fold128(returned_sll_epi16(a, n)),
           fold128(returned_sll_epi32(a, n)), fold128(returned_sll_epi64(a, n)));
    printf("returned srl %llx %llx %llx\n", fold128(returned_srl_epi16(a, n)),
           fold128(returned_srl_epi32(a, n)), fold128(returned_srl_epi64(a, n)));
    printf("returned sra %llx %llx\n", fold128(returned_sra_epi16(a, n)),
           fold128(returned_sra_epi32(a, n)));
    printf("returned avg mul %llx %llx\n", fold128(returned_avg_epu16(a, b)),
           fold128(returned_mul_epi32(a, b)));
    printf("returned 256 %llx %llx %llx\n", fold256(returned_cmpgt_epi8(x, y)),
           fold256(returned_bslli(x)), fold256(returned_bsrli(x)));

    /* The call itself as an argument. */
    printf("passed %llx %llx %llx %llx\n", fold128(_mm_sll_epi16(a, n)),
           fold128(_mm_avg_epu16(a, b)), fold128(_mm_mul_epi32(a, b)),
           fold256(_mm256_cmpgt_epi8(x, y)));
    printf("passed 256 %llx %llx\n", fold256(_mm256_bslli_epi128(x, 3)),
           fold256(_mm256_bsrli_epi128(x, 5)));

    /* One arm of a conditional, a member, the operand of a comma, a cast. */
    __m128i chosen = which ? _mm_sll_epi32(a, n) : _mm_sra_epi32(a, n);
    struct box boxed = { 1, _mm_srl_epi64(a, n) };
    boxed.v = _mm_xor_si128(boxed.v, (which++, _mm_sra_epi16(a, n)));
    __m128 as_float = (__m128)_mm_srl_epi32(a, n);
    printf("placed %llx %llx %llx\n", fold128(chosen), fold128(boxed.v),
           fold128(_mm_castps_si128(as_float)));

    printf("sizes %zu %zu %zu\n", sizeof(_mm_sll_epi16(a, n)), sizeof(_mm_mul_epi32(a, b)),
           sizeof(_mm256_cmpgt_epi8(x, y)));
    return 0;
}
