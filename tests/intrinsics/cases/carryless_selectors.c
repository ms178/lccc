// FLAGS: -mavx2 -mpclmul -mvpclmulqdq
/* Every selector of the carry-less multiply, at 128 and 256 bits.
 *
 * Bit 0 of the immediate selects the half of the first operand, bit 4 the
 * half of the second. The 256-bit intrinsic was a function, so without
 * optimization its selector was a value and the instruction was written
 * with `$0` whatever the program asked for. */
#include <immintrin.h>
#include <stdio.h>
#include <string.h>

static void row(const char *what, __m256i v)
{
    unsigned long long u[4];
    memcpy(u, &v, sizeof u);
    printf("%s %016llx %016llx %016llx %016llx\n", what, u[0], u[1], u[2], u[3]);
}

static void row128(const char *what, __m128i v)
{
    unsigned long long u[2];
    memcpy(u, &v, sizeof u);
    printf("%s %016llx %016llx\n", what, u[0], u[1]);
}

int main(void)
{
    volatile unsigned long long s = 0x0123456789abcdefULL;
    __m256i a = _mm256_set_epi64x(s * 3, s * 5, s * 7, s * 11);
    __m256i b = _mm256_set_epi64x(s * 13, s * 17, s * 19, s * 23);
    __m128i c = _mm256_castsi256_si128(a), d = _mm256_castsi256_si128(b);

    row("256 00", _mm256_clmulepi64_epi128(a, b, 0x00));
    row("256 01", _mm256_clmulepi64_epi128(a, b, 0x01));
    row("256 10", _mm256_clmulepi64_epi128(a, b, 0x10));
    row("256 11", _mm256_clmulepi64_epi128(a, b, 0x11));
    row("256 ee", _mm256_clmulepi64_epi128(a, b, 0xee));
    row("256 ff", _mm256_clmulepi64_epi128(a, b, 0xff));
    row128("128 00", _mm_clmulepi64_si128(c, d, 0x00));
    row128("128 01", _mm_clmulepi64_si128(c, d, 0x01));
    row128("128 10", _mm_clmulepi64_si128(c, d, 0x10));
    row128("128 11", _mm_clmulepi64_si128(c, d, 0x11));
    return 0;
}
