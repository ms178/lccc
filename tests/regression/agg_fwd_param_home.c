/* aggregate_copy_forward store-only forwarding must not treat a register
 * parameter's entry-block home slot as a forwardable source.
 *
 * dump()'s __m128 parameter arrives in %xmm0 and is spilled to its home
 * alloca by the PROLOGUE - there is no IR-visible store. The buggy pass saw
 * "no stores to redirect", deleted the `memcpy tmp, v_home` snapshot anyway,
 * and the copy destination fed printf with uninitialized stack memory
 * (t128_fp/t512_int intrinsic differential failures: all-zero / garbage
 * lanes from the 4th call site on).
 *
 * Needs >=4 distinct call sites so at least one call stays out-of-line
 * after inlining - the inlined bodies never exposed the bug. */
#include <immintrin.h>
#include <stdio.h>
#include <string.h>

static void dump(const char *tag, __m128 v) {
    float out[4];
    _mm_storeu_ps(out, v);
    printf("%s: %g %g %g %g\n", tag, out[0], out[1], out[2], out[3]);
}

int main(void) {
    __m128 a = _mm_set_ps(4.0f, 3.0f, 2.0f, 1.0f);
    __m128 b = _mm_set_ps(2.0f, 2.0f, 2.0f, 2.0f);
    dump("add", _mm_add_ps(a, b));
    dump("sub", _mm_sub_ps(a, b));
    dump("mul", _mm_mul_ps(a, b));
    dump("div", _mm_div_ps(a, b));

    /* self-check without relying on stdout comparison */
    float out[4];
    _mm_storeu_ps(out, _mm_div_ps(a, b));
    if (out[0] != 0.5f || out[1] != 1.0f || out[2] != 1.5f || out[3] != 2.0f)
        return 1;
    _mm_storeu_ps(out, _mm_mul_ps(a, b));
    if (out[0] != 2.0f || out[1] != 4.0f || out[2] != 6.0f || out[3] != 8.0f)
        return 2;
    return 0;
}
