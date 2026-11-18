#include <immintrin.h>
#include <stdio.h>
#include <string.h>
/* Regression: AVX1-only (-mavx, no AVX2) 256-bit integer splats, zero and
 * all-ones vectors, 256-bit to 128-bit casts, and typed float logic
 * (and/or/xor/andnot, ps and pd).
 *
 * The integer splats are written as vector initializers, which both compilers
 * lower to a broadcast; `_mm256_set1_epi*` is AVX2-only and GCC rejects it
 * under -mavx. Every result is folded bitwise into an FNV hash, so any lane
 * that differs from GCC 16.2 (-O2 -mavx) changes the printed value. The logic
 * ops are pure bit operations, so NaN payloads are compared exactly. Inputs
 * reach each kernel through noinline calls, so every splat is a runtime
 * splat, not a folded constant; the *_c kernels cover compile-time splats. */
typedef char v32qi __attribute__((vector_size(32)));
typedef short v16hi __attribute__((vector_size(32)));
typedef int v8si __attribute__((vector_size(32)));
typedef long long v4di __attribute__((vector_size(32)));

#define R8(x) x, x, x, x, x, x, x, x
#define R32(x) R8(x), R8(x), R8(x), R8(x)

__attribute__((noinline)) void s8(void *o, signed char x) {
    v32qi v = {R32(x)};
    memcpy(o, &v, 32);
}
__attribute__((noinline)) void s16(void *o, short x) {
    v16hi v = {R8(x), R8(x)};
    memcpy(o, &v, 32);
}
__attribute__((noinline)) void s32(void *o, int x) {
    v8si v = {R8(x)};
    memcpy(o, &v, 32);
}
__attribute__((noinline)) void s64(void *o, long long x) {
    v4di v = {x, x, x, x};
    memcpy(o, &v, 32);
}
__attribute__((noinline)) void s8_c(void *o) {
    v32qi v = {R32((signed char)0xA5)};
    memcpy(o, &v, 32);
}
__attribute__((noinline)) void s16_c(void *o) {
    v16hi v = {R8((short)0x8001), R8((short)0x8001)};
    memcpy(o, &v, 32);
}
__attribute__((noinline)) void s32_c(void *o) {
    v8si v = {R8(0x7fff1234)};
    memcpy(o, &v, 32);
}
__attribute__((noinline)) void s64_c(void *o) {
    v4di v = {0x0123456789abcdefLL, 0x0123456789abcdefLL,
              0x0123456789abcdefLL, 0x0123456789abcdefLL};
    memcpy(o, &v, 32);
}
__attribute__((noinline)) void ones_c(void *o) {
    v8si v = {R8(-1)};
    memcpy(o, &v, 32);
}
__attribute__((noinline)) void zero_si(void *o) {
    __m256i z = _mm256_setzero_si256();
    memcpy(o, &z, 32);
}
__attribute__((noinline)) void zero_ps(void *o) {
    __m256 z = _mm256_setzero_ps();
    memcpy(o, &z, 32);
}
__attribute__((noinline)) void cast_si(void *o, const void *a) {
    __m256i x;
    memcpy(&x, a, 32);
    __m128i r = _mm256_castsi256_si128(x);
    memcpy(o, &r, 16);
}
__attribute__((noinline)) void cast_ps(void *o, const void *a) {
    __m256 x;
    memcpy(&x, a, 32);
    __m128 r = _mm256_castps256_ps128(x);
    memcpy(o, &r, 16);
}
__attribute__((noinline)) void cast_pd(void *o, const void *a) {
    __m256d x;
    memcpy(&x, a, 32);
    __m128d r = _mm256_castpd256_pd128(x);
    memcpy(o, &r, 16);
}
__attribute__((noinline)) void logic_ps(void *o, const void *a, const void *b) {
    __m256 x, y;
    memcpy(&x, a, 32);
    memcpy(&y, b, 32);
    __m256 r0 = _mm256_and_ps(x, y);
    __m256 r1 = _mm256_or_ps(x, y);
    __m256 r2 = _mm256_xor_ps(x, y);
    __m256 r3 = _mm256_andnot_ps(x, y);
    memcpy(o, &r0, 32);
    memcpy((char *)o + 32, &r1, 32);
    memcpy((char *)o + 64, &r2, 32);
    memcpy((char *)o + 96, &r3, 32);
}
__attribute__((noinline)) void logic_pd(void *o, const void *a, const void *b) {
    __m256d x, y;
    memcpy(&x, a, 32);
    memcpy(&y, b, 32);
    __m256d r0 = _mm256_and_pd(x, y);
    __m256d r1 = _mm256_or_pd(x, y);
    __m256d r2 = _mm256_xor_pd(x, y);
    __m256d r3 = _mm256_andnot_pd(x, y);
    memcpy(o, &r0, 32);
    memcpy((char *)o + 32, &r1, 32);
    memcpy((char *)o + 64, &r2, 32);
    memcpy((char *)o + 96, &r3, 32);
}

static unsigned long long fold(const void *p, unsigned n, unsigned long long h) {
    const unsigned char *c = p;
    for (unsigned i = 0; i < n; i++) h = (h ^ c[i]) * 1099511628211ull;
    return h;
}

int main(void) {
    unsigned char a[32], b[32], o[128];
    unsigned long long h = 1469598103934665603ull;
    unsigned s = 11;
    for (int t = 0; t < 200; t++) {
        for (int i = 0; i < 32; i++) {
            s = s * 1664525u + 1013904223u;
            a[i] = (unsigned char)(s >> 24);
            s = s * 1664525u + 1013904223u;
            b[i] = (unsigned char)(s >> 24);
        }
        /* Runtime scalars, including sign-bit and high-byte cases. */
        signed char c8 = (signed char)a[0];
        short c16 = (short)(a[1] | (a[2] << 8));
        int c32 = (int)((unsigned)a[3] | ((unsigned)a[4] << 8) |
                        ((unsigned)a[5] << 16) | ((unsigned)a[6] << 24));
        long long c64 = ((long long)c32 << 32) ^ (unsigned)(a[7] | (a[8] << 8));
        s8(o, c8);   h = fold(o, 32, h);
        s16(o, c16); h = fold(o, 32, h);
        s32(o, c32); h = fold(o, 32, h);
        s64(o, c64); h = fold(o, 32, h);
        s8_c(o);     h = fold(o, 32, h);
        s16_c(o);    h = fold(o, 32, h);
        s32_c(o);    h = fold(o, 32, h);
        s64_c(o);    h = fold(o, 32, h);
        ones_c(o);   h = fold(o, 32, h);
        zero_si(o);  h = fold(o, 32, h);
        zero_ps(o);  h = fold(o, 32, h);
        cast_si(o, a); h = fold(o, 16, h);
        cast_ps(o, a); h = fold(o, 16, h);
        cast_pd(o, a); h = fold(o, 16, h);
        logic_ps(o, a, b); h = fold(o, 128, h);
        logic_pd(o, a, b); h = fold(o, 128, h);
    }
    printf("avx1_splat_logic %016llx\n", h);
    return 0;
}
