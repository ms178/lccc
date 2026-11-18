/* Inline asm inside a function that also does 128-bit vector work.
 *
 * Under -mavx every 128-bit operation outside the asm body must be VEX-encoded
 * (GCC 16.2 emits no legacy SSE here), and the asm body itself must be left
 * exactly as written.  This fixture checks values, not text: the vector code
 * is interleaved with asm that moves and clobbers xmm registers explicitly, so
 * a wrong promotion of a value that lives across the asm would change the
 * printed result.  Compared byte-for-byte against GCC 16.2.
 *
 * The assembly-level encoding is pinned separately by
 * vex_promote's unit tests (inline_asm_body_is_copied_verbatim and
 * vex_legal_function_with_straight_line_asm_promotes_around_it). */
#include <immintrin.h>
#include <stdio.h>

__attribute__((noinline)) static __m128d mixed_pd(__m128d a, __m128d b, __m128i i, __m128i j)
{
    __asm__ volatile("nop");
    __m128d r = _mm_add_pd(a, b);
    r = _mm_mul_pd(r, b);
    __m128i k = _mm_add_epi32(i, j);
    k = _mm_shuffle_epi32(k, 0x1b);
    k = _mm_unpacklo_epi8(k, j);
    r = _mm_add_pd(r, _mm_cvtepi32_pd(k));
    __asm__ volatile("nop");
    return _mm_add_pd(r, _mm_castsi128_pd(k));
}

/* Explicit xmm traffic in the asm: the value in xmm8 must survive the
 * promoted code around it. */
__attribute__((noinline)) static __m128 explicit_xmm_asm(__m128 x, __m128 y)
{
    __m128 t = _mm_mul_ps(x, y);
    __m128 w;
    /* One asm statement owns xmm8 from entry to exit, so the compiler cannot
     * place its own xmm8 use between the explicit steps. */
    __asm__ volatile("movaps %1, %%xmm8\n\t"
                     "addps %%xmm8, %%xmm8\n\t"
                     "movaps %%xmm8, %0"
                     : "=x"(w)
                     : "x"(t)
                     : "xmm8");
    __m128 u = _mm_sub_ps(x, y);
    return _mm_add_ps(w, u);
}

/* A scalar-only tail after an asm region. */
__attribute__((noinline)) static double scalar_tail(double a, double b)
{
    __m128d v = _mm_set_sd(a);
    __asm__ volatile("nop");
    v = _mm_add_sd(v, _mm_set_sd(b));
    v = _mm_mul_sd(v, _mm_set_sd(0.5));
    return _mm_cvtsd_f64(v);
}

int main(void)
{
    __m128d a = _mm_set_pd(2.5, -1.25);
    __m128d b = _mm_set_pd(0.75, 4.0);
    __m128i i = _mm_set_epi32(7, -3, 11, 2);
    __m128i j = _mm_set_epi32(-5, 9, 1, 100);
    double d[2];
    _mm_storeu_pd(d, mixed_pd(a, b, i, j));
    printf("mixed %.17g %.17g\n", d[0], d[1]);

    float f[4];
    _mm_storeu_ps(f, explicit_xmm_asm(_mm_set_ps(4.0f, 3.0f, 2.0f, 1.0f),
                                       _mm_set_ps(0.5f, -2.0f, 8.0f, 0.25f)));
    printf("explicit %.9g %.9g %.9g %.9g\n", f[0], f[1], f[2], f[3]);

    printf("scalar %.17g\n", scalar_tail(3.0, -0.5));
    return 0;
}
