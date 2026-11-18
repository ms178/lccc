/* Runtime parity for 128-bit and scalar SIMD code under -mavx.
 *
 * Under -mavx every 128-bit and scalar SSE operation must be encoded in VEX
 * form (GCC 16.2 does this).  The encodings differ from the legacy forms in
 * the upper register half, and `movss` register-to-register merges where the
 * VEX three-operand form must be spelled exactly.  Each result is compared
 * with a scalar reference derived from the Intel definition.  Exit status is
 * nonzero on any mismatch.
 */
#include <immintrin.h>
#include <math.h>
#include <stdio.h>
#include <string.h>

static int failures;

static void check_f32(const char *name, const float *got, const float *exp, int n)
{
    if (memcmp(got, exp, (size_t)n * sizeof *got) != 0) {
        printf("FAIL %s\n", name);
        failures++;
    } else {
        printf("ok   %s\n", name);
    }
}

__attribute__((noinline)) static void run_packed(const float *a, const float *b, float *out)
{
    __m128 x = _mm_loadu_ps(a);
    __m128 y = _mm_loadu_ps(b);
    _mm_storeu_ps(out + 0, _mm_add_ps(x, y));
    _mm_storeu_ps(out + 4, _mm_mul_ps(x, y));
    _mm_storeu_ps(out + 8, _mm_shuffle_ps(x, y, 0x1b));
    _mm_storeu_ps(out + 12, _mm_movehl_ps(x, y));
}

__attribute__((noinline)) static void run_move_ss(const float *a, const float *b, float *out)
{
    __m128 x = _mm_loadu_ps(a);
    __m128 y = _mm_loadu_ps(b);
    /* Merge: lanes 1..3 come from x, lane 0 from y. */
    _mm_storeu_ps(out, _mm_move_ss(x, y));
}

__attribute__((noinline)) static float run_scalar(float a, float b)
{
    return a + b;
}

__attribute__((noinline)) static void run_unpack(const double *a, const double *b, double *out)
{
    __m128d x = _mm_loadu_pd(a);
    __m128d y = _mm_loadu_pd(b);
    _mm_storeu_pd(out + 0, _mm_unpackhi_pd(x, y));
    _mm_storeu_pd(out + 2, _mm_movedup_pd(x));
}

__attribute__((noinline)) static void run_int(const int *a, int *out)
{
    __m128i x = _mm_loadu_si128((const __m128i *)a);
    __m128i y = _mm_shuffle_epi32(x, 0x1b);
    _mm_storeu_si128((__m128i *)out, _mm_add_epi32(x, y));
    _mm_storeu_si128((__m128i *)(out + 4), _mm_slli_epi32(x, 3));
}

int main(void)
{
    const float a[4] = {1.0f, 2.0f, 3.0f, 4.0f};
    const float b[4] = {0.5f, -1.0f, 8.0f, 0.25f};
    float got[16], exp[16];

    run_packed(a, b, got);
    for (int i = 0; i < 4; i++) {
        exp[i] = a[i] + b[i];
        exp[4 + i] = a[i] * b[i];
    }
    exp[8] = a[3]; exp[9] = a[2]; exp[10] = b[1]; exp[11] = b[0];
    exp[12] = b[2]; exp[13] = b[3]; exp[14] = a[2]; exp[15] = a[3];
    check_f32("packed_add_mul_shuffle_movehl", got, exp, 16);

    run_move_ss(a, b, got);
    exp[0] = b[0]; exp[1] = a[1]; exp[2] = a[2]; exp[3] = a[3];
    check_f32("move_ss_merge", got, exp, 4);

    float s = run_scalar(a[0], b[0]);
    float s_exp = a[0] + b[0];
    check_f32("scalar_add_ss", &s, &s_exp, 1);

    double d[2] = {1.5, -2.5}, e[2] = {4.0, 0.75}, dg[4], dg_exp[4];
    run_unpack(d, e, dg);
    dg_exp[0] = d[1]; dg_exp[1] = e[1]; dg_exp[2] = d[0]; dg_exp[3] = d[0];
    if (memcmp(dg, dg_exp, sizeof dg) != 0) {
        printf("FAIL unpack_movedup_pd\n");
        failures++;
    } else {
        printf("ok   unpack_movedup_pd\n");
    }

    int ia[4] = {1, -2, 3, 40}, io[8], io_exp[8];
    run_int(ia, io);
    for (int i = 0; i < 4; i++) {
        io_exp[i] = ia[i] + ia[3 - i];
        io_exp[4 + i] = ia[i] * 8; /* == slli 3 without shifting a negative */
    }
    if (memcmp(io, io_exp, sizeof io) != 0) {
        printf("FAIL int_shuffle_add_slli\n");
        failures++;
    } else {
        printf("ok   int_shuffle_add_slli\n");
    }

    if (failures) {
        printf("FAILURES: %d\n", failures);
        return 1;
    }
    printf("PASS\n");
    return 0;
}
