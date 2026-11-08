/* Round 8 stage-2 three-way differential: LCCC / GCC 16.2 / Clang 23.1.5.
 * Exercises the new ISPC transplants at C level with constant masks:
 *   - 512-bit all-off / all-on zeroing loads
 *   - 512-bit and 128-bit merge loads (all-off / all-on)
 *   - 256-bit all-on zeroing load (must NOT be rewritten to a 256-bit
 *     unmasked load: `Loadu256` reads a vector home, not a raw pointer)
 *   - true-prefix masked stores (ISPC lCheckMask narrowing)
 *   - masked arithmetic all-off / all-on
 * Build: -O2 -mavx512f -mavx512bw -mavx512vl -mavx512dq
 */
#include <stdio.h>
#include <string.h>
#include <immintrin.h>

static unsigned char src[64];
static unsigned char dst[64];

static unsigned h(const unsigned char *p, int n) {
    unsigned x = 2166136261u;
    for (int i = 0; i < n; i++) x = (x ^ p[i]) * 16777619u;
    return x;
}

__attribute__((noinline)) static unsigned z512_off(void) {
    memset(dst, 0xA5, 64);
    __m512i v = _mm512_maskz_loadu_epi8((__mmask64)0, src);
    _mm512_storeu_si512(dst, v);
    return h(dst, 64);
}
__attribute__((noinline)) static unsigned z512_on(void) {
    memset(dst, 0xA5, 64);
    __m512i v = _mm512_maskz_loadu_epi8((__mmask64)-1, src);
    _mm512_storeu_si512(dst, v);
    return h(dst, 64);
}
__attribute__((noinline)) static unsigned z256_on(void) {
    memset(dst, 0xA5, 64);
    __m256i v = _mm256_maskz_loadu_epi8((__mmask32)-1, src);
    _mm256_storeu_si256((__m256i_u *)dst, v);
    return h(dst, 32);
}
__attribute__((noinline)) static unsigned m512_off(void) {
    memset(dst, 0xA5, 64);
    __m512i old = _mm512_set1_epi8((char)0x3C);
    __m512i v = _mm512_mask_loadu_epi8(old, (__mmask64)0, src);
    _mm512_storeu_si512(dst, v);
    return h(dst, 64);
}
__attribute__((noinline)) static unsigned m512_on(void) {
    memset(dst, 0xA5, 64);
    __m512i old = _mm512_set1_epi8((char)0x3C);
    __m512i v = _mm512_mask_loadu_epi8(old, (__mmask64)-1, src);
    _mm512_storeu_si512(dst, v);
    return h(dst, 64);
}
__attribute__((noinline)) static unsigned m128_off(void) {
    memset(dst, 0xA5, 64);
    __m128i old = _mm_set1_epi8((char)0x5A);
    __m128i v = _mm_mask_loadu_epi8(old, (__mmask16)0, src);
    _mm_storeu_si128((__m128i_u *)dst, v);
    return h(dst, 16);
}
__attribute__((noinline)) static unsigned m128_on(void) {
    memset(dst, 0xA5, 64);
    __m128i old = _mm_set1_epi8((char)0x5A);
    __m128i v = _mm_mask_loadu_epi8(old, (__mmask16)-1, src);
    _mm_storeu_si128((__m128i_u *)dst, v);
    return h(dst, 16);
}
__attribute__((noinline)) static unsigned p128_store(void) {
    memset(dst, 0x11, 64);
    __m128i v = _mm_set1_epi8((char)0x7E);
    _mm_mask_storeu_epi8(dst, (__mmask16)0x0007, v); /* prefix of 3 */
    return h(dst, 16);
}
__attribute__((noinline)) static unsigned p512_store(void) {
    memset(dst, 0x11, 64);
    __m512i v = _mm512_set1_epi8((char)0x42);
    _mm512_mask_storeu_epi8(dst, (__mmask64)0x1FF, v); /* prefix of 9 */
    return h(dst, 16);
}
__attribute__((noinline)) static unsigned s512_off(void) {
    memset(dst, 0xA5, 64);
    __m512i v = _mm512_maskz_set1_epi16((__mmask32)0, 0x1234);
    _mm512_storeu_si512(dst, v);
    return h(dst, 64);
}
__attribute__((noinline)) static unsigned s512_on(void) {
    memset(dst, 0xA5, 64);
    __m512i v = _mm512_maskz_set1_epi16((__mmask32)-1, 0x1234);
    _mm512_storeu_si512(dst, v);
    return h(dst, 64);
}
__attribute__((noinline)) static unsigned a512_off(void) {
    memset(dst, 0xA5, 64);
    __m512i a = _mm512_set1_epi8((char)1);
    __m512i b = _mm512_set1_epi8((char)2);
    __m512i v = _mm512_maskz_maddubs_epi16((__mmask32)0, a, b);
    _mm512_storeu_si512(dst, v);
    return h(dst, 64);
}
__attribute__((noinline)) static unsigned a512_on(void) {
    memset(dst, 0xA5, 64);
    __m512i a = _mm512_set1_epi8((char)1);
    __m512i b = _mm512_set1_epi8((char)2);
    __m512i v = _mm512_maskz_maddubs_epi16((__mmask32)-1, a, b);
    _mm512_storeu_si512(dst, v);
    return h(dst, 64);
}

/* A STACK destination is bounds-provable, so ISPC `lCheckMask` can narrow
 * the true-prefix masked store into an exact n-byte unmasked copy.  The
 * global forms above deliberately refuse (their size is not in the IR). */
__attribute__((noinline)) static unsigned p128_stack_store(void) {
    unsigned char local[64];
    for (int i = 0; i < 64; i++) local[i] = 0x11;
    __m128i v = _mm_set1_epi8((char)0x7E);
    _mm_mask_storeu_epi8(local, (__mmask16)0x0007, v); /* prefix of 3 */
    return h(local, 16);
}
__attribute__((noinline)) static unsigned p512_stack_store(void) {
    unsigned char local[64];
    for (int i = 0; i < 64; i++) local[i] = 0x11;
    __m512i v = _mm512_set1_epi8((char)0x42);
    _mm512_mask_storeu_epi8(local, (__mmask64)0x1FF, v); /* prefix of 9 */
    return h(local, 16);
}

int main(void) {
    for (int i = 0; i < 64; i++) src[i] = (unsigned char)(i * 5 + 3);
    printf("%u %u %u %u %u %u %u %u %u %u %u %u %u %u %u\n",
           z512_off(), z512_on(), z256_on(),
           m512_off(), m512_on(), m128_off(), m128_on(),
           p128_store(), p512_store(), p128_stack_store(), p512_stack_store(),
           s512_off(), s512_on(), a512_off(), a512_on());
    return 0;
}
