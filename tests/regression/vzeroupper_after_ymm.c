/* IS-20: vzeroupper at function exit after 256-bit vector use.
 *
 * Requires -mavx2 (see the .flags file). A function that executes any
 * YMM instruction must clean the upper halves before returning so legacy
 * SSE callers do not pay the AVX-SSE transition penalty. Runtime check:
 * the vector computation itself plus a legacy-SSE consumer afterwards. */
#include <immintrin.h>
#include <stdio.h>

__attribute__((noinline)) void copy64(void *dst, const void *src) {
    __m256i a = _mm256_loadu_si256((const __m256i *)src);
    __m256i b = _mm256_loadu_si256((const __m256i *)src + 1);
    _mm256_storeu_si256((__m256i *)dst, a);
    _mm256_storeu_si256((__m256i *)dst + 1, b);
}

__attribute__((noinline)) double legacy_sse(double x) {
    __m128d v = _mm_set_sd(x);
    v = _mm_add_sd(v, _mm_set_sd(1.0));
    return _mm_cvtsd_f64(v);
}

int main(void) {
    alignas(32) unsigned char src[64], dst[64];
    for (int i = 0; i < 64; i++) src[i] = (unsigned char)(i * 3);
    copy64(dst, src);
    for (int i = 0; i < 64; i++)
        if (dst[i] != src[i]) return 1;
    /* Legacy SSE consumer immediately after the AVX caller: with the
     * vzeroupper in place this cannot take a transition-stall path that
     * corrupts xmm state (and the value must be exact regardless). */
    if (legacy_sse(41.0) != 42.0) return 2;
    return 0;
}
