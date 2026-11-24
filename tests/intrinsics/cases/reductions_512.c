// FLAGS: -mavx512f
/* The 512-bit reductions.
 *
 * `_mm512_reduce_add_epi64` shared the 32-bit reduction and returned the
 * sum of the sixteen halves. `_mm512_reduce_add_ps` was a running sum from
 * element 0; it is defined as a sum by halves (upper half onto lower half,
 * then again), and floating-point addition rounds, so the order is part of
 * the result. The floats below are chosen so that the two orders differ. */
#include <immintrin.h>
#include <stdio.h>
#include <string.h>

int main(void)
{
    volatile long long words[8] = { 1, 2, 3, 4, 5, 6, 7, 0x7000000000000000LL };
    volatile long long carries[8] = { 0xffffffffLL, 0xffffffffLL, 1, 1, 0x100000000LL, -1, -2, 0x7fffffffLL };
    volatile float floats[16] = { 1e8f, 1.0f, -1e8f, 1.0f, 3.0f, 1e-3f, 16777216.0f, 1.0f,
                                  1.0f, -16777216.0f, 0.5f, 0.25f, 1e20f, -1e20f, 7.0f, 1e-7f };
    volatile float small[16] = { 0.1f, 0.2f, 0.3f, 0.4f, 0.5f, 0.6f, 0.7f, 0.8f,
                                 0.9f, 1.0f, 1.1f, 1.2f, 1.3f, 1.4f, 1.5f, 1.6f };
    __m512i a = _mm512_loadu_si512((const void *)words);
    __m512i b = _mm512_loadu_si512((const void *)carries);
    float sum;
    unsigned bits;

    printf("epi64 %lld %lld\n", _mm512_reduce_add_epi64(a), _mm512_reduce_add_epi64(b));
    printf("epi32 %d %d\n", _mm512_reduce_add_epi32(a), _mm512_reduce_add_epi32(b));

    sum = _mm512_reduce_add_ps(_mm512_loadu_ps((const void *)floats));
    memcpy(&bits, &sum, sizeof bits);
    printf("ps %08x\n", bits);
    sum = _mm512_reduce_add_ps(_mm512_loadu_ps((const void *)small));
    memcpy(&bits, &sum, sizeof bits);
    printf("ps %08x\n", bits);
    return 0;
}
