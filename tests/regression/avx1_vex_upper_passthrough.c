/* Runtime guard for the VEX-legal promotion of functions without %ymm.
 *
 * A function that never names %ymm can still be the carrier of a 256-bit
 * value: it forwards an incoming __m256 to an external callee (a tail or
 * ordinary call), or it rewrites only the low 128 bits and then calls out.
 * Promoting its legacy SSE to VEX is exact for bits 127:0.  The upper half is
 * preserved only if the promotion never clobbers state that a callee or a
 * later reader still uses.  Both cases below must return the full 256 bits
 * unchanged, and the output must match GCC 16.2 bit for bit.
 */
#include <immintrin.h>
#include <stdio.h>

extern __m256 keep_upper(__m256 v); /* external to this file: may read all of ymm */

/* No %ymm in this body: the value is only forwarded to the callee. */
__attribute__((noinline)) __m256 fwd_tail(__m256 v)
{
    return keep_upper(v);
}

/* 128-bit legacy-style work on the low half, then a call that must see the
 * untouched upper half of the reassembled value. */
__attribute__((noinline)) __m256 low_then_call(__m256 v)
{
    __m128 lo = _mm256_castps256_ps128(v);
    lo = _mm_add_ps(lo, lo);
    __m256 w = _mm256_insertf128_ps(v, lo, 0);
    return keep_upper(w);
}

__m256 keep_upper(__m256 v)
{
    return v;
}

static int check(const char *name, const float *got, const float *exp)
{
    for (int i = 0; i < 8; i++) {
        if (got[i] != exp[i]) {
            printf("FAIL %s lane %d: got %g expected %g\n", name, i, got[i], exp[i]);
            return 1;
        }
    }
    printf("ok   %s\n", name);
    return 0;
}

int main(void)
{
    const __m256 a = _mm256_set_ps(8, 7, 6, 5, 4, 3, 2, 1);
    float got[8];
    int fails = 0;

    const float fwd_exp[8] = {1, 2, 3, 4, 5, 6, 7, 8};
    _mm256_storeu_ps(got, fwd_tail(a));
    fails += check("fwd_tail", got, fwd_exp);

    /* Low half doubled (1,2,3,4 -> 2,4,6,8); upper half 5..8 unchanged. */
    const float low_exp[8] = {2, 4, 6, 8, 5, 6, 7, 8};
    _mm256_storeu_ps(got, low_then_call(a));
    fails += check("low_then_call", got, low_exp);

    if (fails) {
        printf("FAILURES: %d\n", fails);
        return 1;
    }
    printf("PASS\n");
    return 0;
}
