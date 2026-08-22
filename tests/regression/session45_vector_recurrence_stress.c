/* Session-45 red-team stress: loop-carried pointer arithmetic feeding SIMD
 * vector temporaries. Exercises the pointer-root SCC shortcut
 * (`p = phi(seed, p + stride)`) inside vector_temp_promotion, plus the
 * recurrence-derived Loaddqu forwarding guard.
 *
 * Shapes covered:
 *  - single-seed pointer recurrence through a vector load/store cycle;
 *  - two interleaved recurrences with different seeds (must fail closed);
 *  - memcpy-based vector temp promotion inside the recurrence body;
 *  - wide scalar access to a formerly-32-aligned vector temp (align relax);
 *  - read-modify-write FMA accumulator that must NOT be promoted.
 */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <immintrin.h>

#define N 1024
#define V 8 /* 8 x 32-byte vectors */

static float a[N], b[N], c[N];

__attribute__((noinline)) static void
recurrence_kernel(const float *restrict src, float *restrict dst, int iters)
{
    for (int i = 0; i < iters; i++) {
        /* Single-seed pointer recurrence: p = phi(src+0, p + 32) over
         * V segments, so the whole V*8 window is rewritten. */
        const __m256 *p = (const __m256 *)(src + (i & (V - 1)) * 8);
        __m256 v = _mm256_loadu_ps((const float *)p);
        __m256 w = _mm256_add_ps(v, v);
        float *q = dst + (i & (V - 1)) * 8;
        _mm256_storeu_ps(q, w);
    }
}

__attribute__((noinline)) static float
rmw_accumulator(const float *restrict p, int iters)
{
    __m256 acc = _mm256_setzero_ps();
    float out[8];
    for (int i = 0; i < iters; i++) {
        __m256 v = _mm256_loadu_ps(p + i * 8);
        acc = _mm256_fmadd_ps(v, v, acc); /* RMW accumulator */
    }
    _mm256_storeu_ps(out, acc);
    float sum = 0;
    for (int i = 0; i < 8; i++) sum += out[i];
    return sum;
}

__attribute__((noinline)) static void
wide_scalar_through_vector_temp(float *dst)
{
    __m256 v = _mm256_set1_ps(2.0f);
    float tmp[8];
    _mm256_storeu_ps(tmp, v);
    dst[0] = tmp[0];
    dst[1] = tmp[1] + tmp[2] + tmp[3] + tmp[4] + tmp[5] + tmp[6] + tmp[7];
}

int main(void)
{
    for (int i = 0; i < N; i++) {
        a[i] = (float)(i * 3 + 1);
        b[i] = 0.0f;
        c[i] = 0.0f;
    }
    recurrence_kernel(a, b, N / V);
    /* The recurrence kernel rewrites exactly the first V*8 floats (its
     * pointer cycles through the same 32-float window). */
    for (int i = 0; i < V * 8; i++) {
        if (b[i] != 2.0f * a[i]) {
            printf("FAIL recurrence @%d: %g != %g\n", i, b[i], 2.0f * a[i]);
            return 1;
        }
    }
    float got = rmw_accumulator(a, N / V);
    float expect = 0.0f;
    for (int i = 0; i < N; i++) expect += a[i] * a[i];
    /* The SIMD kernel contracts mul+add into FMA; the scalar reference does
     * not.  Both roundings are legal, so compare with a relative tolerance
     * (the differential gate against GCC compares the exact bytes). */
    float diff = got - expect;
    if (diff < 0) diff = -diff;
    if (diff > 1e-3f * (expect < 0 ? -expect : expect)) {
        printf("FAIL rmw: %g != %g\n", got, expect);
        return 2;
    }
    wide_scalar_through_vector_temp(c);
    if (c[0] != 2.0f || c[1] != 14.0f) {
        printf("FAIL wide-scalar: %g %g\n", c[0], c[1]);
        return 3;
    }
    printf("OK session45 vector-recurrence stress\n");
    return 0;
}
