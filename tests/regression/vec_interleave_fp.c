#include <stdio.h>
#include <stdlib.h>
#include <math.h>

/* FP reduction shapes for vec_interleave with -ffast-math.  Interleaving
 * reassociates the FP sum, so the result legitimately differs from GCC's
 * single-chain order by a few ulps; the oracle compare is disabled and the
 * test gates on its own tolerance check against an exact ordered scalar
 * reference (volatile accumulator = strictly ordered, unvectorisable). */

static double dot(const double *a, const double *b, int n) {
    double s = 0;
    for (int i = 0; i < n; i++) s += a[i] * b[i];
    return s;
}

static double sumv(const double *a, int n) {
    double s = 0;
    for (int i = 0; i < n; i++) s += a[i];
    return s;
}

static float dotf(const float *a, const float *b, int n) {
    float s = 0;
    for (int i = 0; i < n; i++) s += a[i] * b[i];
    return s;
}

static int check(int n, const double *a, const double *b, const float *fa, const float *fb) {
    volatile double rd = 0, rs = 0;
    volatile float rf = 0;
    for (int i = 0; i < n; i++) rd += a[i] * b[i];
    for (int i = 0; i < n; i++) rs += a[i];
    for (int i = 0; i < n; i++) rf += fa[i] * fb[i];
    double d = dot(a, b, n);
    double s = sumv(a, n);
    float f = dotf(fa, fb, n);
    if (fabs(d - rd) > 1e-9 * fmax(fabs(rd), 1.0)) {
        printf("FAIL dot n=%d got=%.12g ref=%.12g\n", n, d, rd);
        return 1;
    }
    if (fabs(s - rs) > 1e-9 * fmax(fabs(rs), 1.0)) {
        printf("FAIL sum n=%d got=%.12g ref=%.12g\n", n, s, rs);
        return 1;
    }
    if (fabsf(f - (float)rf) > 1e-3f * fmaxf(fabsf((float)rf), 1.0f)) {
        printf("FAIL dotf n=%d got=%.9g ref=%.9g\n", n, (double)f, (double)rf);
        return 1;
    }
    return 0;
}

int main(void) {
    int n = 20000;
    double *a = (double *)malloc((size_t)n * 8);
    double *b = (double *)malloc((size_t)n * 8);
    float *fa = (float *)malloc((size_t)n * 4);
    float *fb = (float *)malloc((size_t)n * 4);
    if (!a || !b || !fa || !fb) return 2;
    for (int i = 0; i < n; i++) {
        a[i] = (i % 17) * 0.5 - 3.0;
        b[i] = (i % 19) * 0.25 + 1.0;
        fa[i] = (i % 13) * 0.5f - 2.0f;
        fb[i] = (i % 23) * 0.25f + 0.5f;
    }
    /* Sweep small sizes (main loop skipped), boundary sizes around vector
     * multiples, and the full size. */
    int probe[] = {0, 1, 2, 3, 4, 7, 8, 15, 16, 31, 32, 33, 63, 64, 65, 127, 128, 129, 255, 256, 257, 1000, 1001, n};
    for (unsigned i = 0; i < sizeof(probe) / sizeof(probe[0]); i++) {
        if (check(probe[i], a, b, fa, fb)) return 1;
    }
    printf("all OK\n");
    return 0;
}
