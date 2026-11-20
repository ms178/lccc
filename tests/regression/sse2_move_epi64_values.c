/* SSE2 _mm_move_epi64 (MOVQ xmm, xmm) value fixture.
 *
 * _mm_move_epi64 keeps the low 64-bit lane of its operand and zeroes the high
 * 64-bit lane. The name was missing from lccc's SSE2 headers (GCC 16.2 accepts
 * it). Every case uses a high lane that is non-zero so a missing zeroing, a
 * swapped lane, or a lane taken from the wrong operand changes the printed
 * output. The binary must print the same bytes as GCC 16.2. */
#include <emmintrin.h>
#include <stdio.h>

static void dump(const char *tag, __m128i v)
{
    long long r[2];
    _mm_storeu_si128((__m128i *)r, v);
    printf("%s %lld %lld\n", tag, r[0], r[1]);
}

int main(void)
{
    long long A[2] = {0x0123456789abcdefLL, -0x7edcba9876543210LL};
    long long Z[2] = {0, 0};
    long long N[2] = {-1, -1};
    __m128i a = _mm_loadu_si128((const __m128i *)A);

    dump("move_a", _mm_move_epi64(a));
    dump("move_zero", _mm_move_epi64(_mm_loadu_si128((const __m128i *)Z)));
    dump("move_ones", _mm_move_epi64(_mm_loadu_si128((const __m128i *)N)));

    /* The result must be independent of the high lane of the input. */
    long long H[2] = {42, 0x7fffffffffffffffLL};
    dump("move_high", _mm_move_epi64(_mm_loadu_si128((const __m128i *)H)));
    return 0;
}
