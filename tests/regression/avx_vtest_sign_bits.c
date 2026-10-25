/* VTEST observes sign bits only, including for NaNs and nonzero payloads.
 * Exhaust every pair of eight-lane sign masks; shorter forms project them.
 * AVX, not AVX2, is the minimum ISA for every operation below. */
#include <immintrin.h>
#include <stdio.h>

typedef float v4sf __attribute__((vector_size(16)));
typedef float v8sf __attribute__((vector_size(32)));
typedef double v2df __attribute__((vector_size(16)));
typedef double v4df __attribute__((vector_size(32)));

static int wrong(int z, int c, int nzc, unsigned a, unsigned b, unsigned mask)
{
    int ez = (a & b & mask) == 0;
    int ec = (~a & b & mask) == 0;
    return z != ez || c != ec || nzc != (!ez && !ec);
}

__attribute__((noinline)) static int check(unsigned a, unsigned b)
{
    union { unsigned u[8]; float f[8]; v4sf v4; v8sf v8; } x, y;
    union { unsigned long long u[4]; double d[4]; v2df v2; v4df v4; } p, q;
    for (unsigned i = 0; i < 8; ++i) {
        /* sNaN/qNaN payloads must not affect flags or raise invalid. */
        x.u[i] = (0x7f800001u + i) | (((a >> i) & 1u) << 31);
        y.u[i] = (0x7fc00021u + i) | (((b >> i) & 1u) << 31);
    }
    for (unsigned i = 0; i < 4; ++i) {
        p.u[i] = (0x7ff0000000000001ull + i) | ((unsigned long long)((a >> i) & 1u) << 63);
        q.u[i] = (0x7ff8000000000021ull + i) | ((unsigned long long)((b >> i) & 1u) << 63);
    }
    __m128 xf = _mm_loadu_ps(x.f), yf = _mm_loadu_ps(y.f);
    __m256 xw = _mm256_loadu_ps(x.f), yw = _mm256_loadu_ps(y.f);
    __m128d pd = _mm_loadu_pd(p.d), qd = _mm_loadu_pd(q.d);
    __m256d pw = _mm256_loadu_pd(p.d), qw = _mm256_loadu_pd(q.d);
#define TEST(PREFIX, SUFFIX, A, B, MASK) \
    wrong(PREFIX##_testz_##SUFFIX(A, B), \
          PREFIX##_testc_##SUFFIX(A, B), \
          PREFIX##_testnzc_##SUFFIX(A, B), a, b, MASK)
    if (TEST(_mm, ps, xf, yf, 15) || TEST(_mm256, ps, xw, yw, 255) ||
        TEST(_mm, pd, pd, qd, 3) || TEST(_mm256, pd, pw, qw, 15))
        return 1;
#undef TEST
#define RAW(SUFFIX, A, B, MASK) \
    wrong(__builtin_ia32_vtestz##SUFFIX(A, B), \
          __builtin_ia32_vtestc##SUFFIX(A, B), \
          __builtin_ia32_vtestnzc##SUFFIX(A, B), a, b, MASK)
    if (RAW(ps, x.v4, y.v4, 15) || RAW(ps256, x.v8, y.v8, 255) ||
        RAW(pd, p.v2, q.v2, 3) || RAW(pd256, p.v4, q.v4, 15))
        return 2;
#undef RAW
    return 0;
}

int main(void)
{
    unsigned saved = _mm_getcsr();
    _mm_setcsr((saved | 0x1f80u) & ~63u);
    for (unsigned a = 0; a < 256; ++a)
        for (unsigned b = 0; b < 256; ++b)
            if (check(a, b)) {
                printf("VTEST mismatch %u %u\n", a, b);
                _mm_setcsr(saved);
                return 1;
            }
    unsigned exceptions = _mm_getcsr() & 63u;
    _mm_setcsr(saved);
    if (exceptions) {
        printf("VTEST raised FP exceptions: %u\n", exceptions);
        return 2;
    }
    puts("AVX VTEST public/native builtins: all sign masks passed");
    return 0;
}
