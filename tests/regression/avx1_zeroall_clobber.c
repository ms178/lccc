/* _mm256_zeroall (vzeroall) under -mavx.
 *
 * vzeroall destroys every YMM register, so a 256-bit value live across the
 * call must be spilled and reloaded.  Each check keeps a vector live across
 * the call and compares the stored result with a value derived from the
 * input, not from another compiler's output.  Exit status is nonzero on any
 * mismatch.
 */
#include <immintrin.h>
#include <stdio.h>
#include <string.h>

static int failures;

static void check_floats(const char *name, const float *got, const float *exp,
                         size_t n)
{
    if (memcmp(got, exp, n * sizeof *got) != 0) {
        printf("FAIL %s\n", name);
        failures++;
    } else {
        printf("ok   %s\n", name);
    }
}

/* A YMM value loaded before the call and used after it. */
__attribute__((noinline)) static void add_across_zeroall(const float *src,
                                                         float *dst)
{
    __m256 v = _mm256_loadu_ps(src);
    _mm256_zeroall();
    _mm256_storeu_ps(dst, _mm256_add_ps(v, v));
}

/* Two YMM values live across the call, combined after it. */
__attribute__((noinline)) static void mul_across_zeroall(const float *a,
                                                         const float *b,
                                                         float *dst)
{
    __m256 x = _mm256_loadu_ps(a);
    __m256 y = _mm256_loadu_ps(b);
    _mm256_zeroall();
    _mm256_storeu_ps(dst, _mm256_mul_ps(x, y));
}

/* zeroall with no vector live across it: the call alone must not affect the
 * value stored afterwards. */
__attribute__((noinline)) static void store_after_zeroall(float *dst)
{
    _mm256_zeroall();
    _mm256_storeu_ps(dst, _mm256_set1_ps(3.0f));
}

int main(void)
{
    static const float a[8] = {1.0f, 2.0f, 3.0f, 4.0f, 5.0f, 6.0f, 7.0f, 8.0f};
    static const float b[8] = {2.0f, 2.0f, 2.0f, 2.0f, 0.5f, 0.5f, 0.5f, 0.5f};
    float got[8];
    float exp[8];

    /* add: exp[i] = a[i] + a[i]. */
    for (int i = 0; i < 8; i++) {
        exp[i] = a[i] + a[i];
    }
    add_across_zeroall(a, got);
    check_floats("add_across_zeroall", got, exp, 8);

    /* mul: exp[i] = a[i] * b[i]. */
    for (int i = 0; i < 8; i++) {
        exp[i] = a[i] * b[i];
    }
    mul_across_zeroall(a, b, got);
    check_floats("mul_across_zeroall", got, exp, 8);

    /* store-only: every lane is 3.0. */
    for (int i = 0; i < 8; i++) {
        exp[i] = 3.0f;
    }
    store_after_zeroall(got);
    check_floats("store_after_zeroall", got, exp, 8);

    if (failures) {
        printf("FAILURES: %d\n", failures);
        return 1;
    }
    printf("PASS\n");
    return 0;
}
