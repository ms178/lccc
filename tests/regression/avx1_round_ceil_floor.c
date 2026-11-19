/* AVX1 ceil/floor and rounding-mode macro fixture.
 *
 * _mm256_ceil_{ps,pd} and _mm256_floor_{ps,pd} expand to _mm256_round_* with
 * _MM_FROUND_CEIL / _MM_FROUND_FLOOR.  lccc's smmintrin.h did not define those
 * constants, so under -mavx the ceil/floor forms failed to compile while GCC
 * 16.2 accepts them.  The constants are now defined with GCC's exact values.
 *
 * Every lane is checked against a scalar reference (self-checking binary), and
 * the constant values are printed so the output can be compared with GCC 16.2.
 * Built with -O2 -mavx (see avx1_round_ceil_floor.flags). */
#include <immintrin.h>
#include <stdio.h>

static long long ceil_ref_d(double v)
{
    long long t = (long long)v;
    return (double)t < v ? t + 1 : t;
}

static long long floor_ref_d(double v)
{
    long long t = (long long)v;
    return (double)t > v ? t - 1 : t;
}

static long long ceil_ref_f(float v)
{
    long long t = (long long)v;
    return (float)t < v ? t + 1 : t;
}

static long long floor_ref_f(float v)
{
    long long t = (long long)v;
    return (float)t > v ? t - 1 : t;
}

static int check_d(const char *what, const double *got, const long long *want, int n)
{
    int bad = 0;
    printf("%s:", what);
    for (int i = 0; i < n; i++) {
        printf(" %lld", (long long)got[i]);
        if ((long long)got[i] != want[i])
            bad++;
    }
    printf(bad ? "  MISMATCH\n" : "  ok\n");
    return bad;
}

static int check_f(const char *what, const float *got, const long long *want, int n)
{
    int bad = 0;
    printf("%s:", what);
    for (int i = 0; i < n; i++) {
        printf(" %lld", (long long)got[i]);
        if ((long long)got[i] != want[i])
            bad++;
    }
    printf(bad ? "  MISMATCH\n" : "  ok\n");
    return bad;
}

int main(void)
{
    /* Covers negative, positive, zero, exact integers, and halves. */
    const double d[8] = {-2.5, -1.0, -0.5, 0.0, 0.5, 1.25, 2.75, -7.9};
    const float f[8] = {-2.5f, -1.0f, -0.5f, 0.0f, 0.5f, 1.25f, 2.75f, -7.9f};
    long long want_c_d[8], want_f_d[8], want_c_f[8], want_f_f[8];
    for (int i = 0; i < 8; i++) {
        want_c_d[i] = ceil_ref_d(d[i]);
        want_f_d[i] = floor_ref_d(d[i]);
        want_c_f[i] = ceil_ref_f(f[i]);
        want_f_f[i] = floor_ref_f(f[i]);
    }

    int bad = 0;
    double out_d[8];
    float out_f[8];

    /* 256-bit ceil/floor: the AVX1 names this fixture exists for. */
    for (int half = 0; half < 2; half++) {
        __m256d vd = _mm256_loadu_pd(d + 4 * half);
        _mm256_storeu_pd(out_d + 4 * half, _mm256_ceil_pd(vd));
    }
    bad += check_d("ceil_pd256", out_d, want_c_d, 8);

    for (int half = 0; half < 2; half++) {
        __m256d vd = _mm256_loadu_pd(d + 4 * half);
        _mm256_storeu_pd(out_d + 4 * half, _mm256_floor_pd(vd));
    }
    bad += check_d("floor_pd256", out_d, want_f_d, 8);

    _mm256_storeu_ps(out_f, _mm256_ceil_ps(_mm256_loadu_ps(f)));
    bad += check_f("ceil_ps256", out_f, want_c_f, 8);

    _mm256_storeu_ps(out_f, _mm256_floor_ps(_mm256_loadu_ps(f)));
    bad += check_f("floor_ps256", out_f, want_f_f, 8);

    /* Explicit rounding-mode forms through _mm256_round_*. */
    _mm256_storeu_pd(out_d, _mm256_round_pd(_mm256_loadu_pd(d), _MM_FROUND_TO_ZERO));
    _mm256_storeu_pd(out_d + 4, _mm256_round_pd(_mm256_loadu_pd(d + 4), _MM_FROUND_TO_ZERO));
    {
        long long want_t[8];
        for (int i = 0; i < 8; i++)
            want_t[i] = (long long)d[i];
        bad += check_d("trunc_pd256", out_d, want_t, 8);
    }

    /* SSE4.1 forms that also use the constants (128-bit, same header). */
    {
        float sf[4];
        double sd[2];
        _mm_storeu_ps(sf, _mm_round_ps(_mm_loadu_ps(f), _MM_FROUND_FLOOR));
        long long want_sf[4] = {floor_ref_f(f[0]), floor_ref_f(f[1]), floor_ref_f(f[2]), floor_ref_f(f[3])};
        bad += check_f("floor_ps128", sf, want_sf, 4);

        _mm_storeu_pd(sd, _mm_round_pd(_mm_loadu_pd(d), _MM_FROUND_CEIL));
        long long want_sd[2] = {ceil_ref_d(d[0]), ceil_ref_d(d[1])};
        bad += check_d("ceil_pd128", sd, want_sd, 2);
    }

    printf("constants: TO_ZERO=%d CEIL=%d FLOOR=%d NINT=%d TRUNC=%d RINT=%d NEARBYINT=%d\n",
           _MM_FROUND_TO_ZERO, _MM_FROUND_CEIL, _MM_FROUND_FLOOR, _MM_FROUND_NINT,
           _MM_FROUND_TRUNC, _MM_FROUND_RINT, _MM_FROUND_NEARBYINT);
    printf("failures %d\n", bad);
    return bad ? 1 : 0;
}
