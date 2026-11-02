/* Floating point: conversions, fused-multiply-add, division by constants,
 * and the strict-aliasing and rounding corners.  Every result is printed as
 * a bit pattern, so a one-ULP difference is visible and cannot be hidden by
 * a printf format.  Results are rounded once before they are examined. */
#include <stdio.h>
#include <stdint.h>
#include <string.h>

static double poly(double x) { return ((x * x + 3.0) * x - 2.0) * x + 1.0; }
static float  fpoly(float x)  { return ((x * x + 3.0f) * x - 2.0f) * x + 1.0f; }
static double fma_like(double a, double b, double c) { return a * b + c; }
static double inv(double x) { return 1.0 / x; }        /* x is never 0 */
static double dpowish(double b, int e) {
    double r = 1.0;
    while (e) { if (e & 1) r *= b; b *= b; e >>= 1; }  /* square-and-multiply */
    return r;
}
static double mix(const double *a, const double *b, int n) {
    double s0 = 0, s1 = 0;
    for (int i = 0; i < n; i += 2) { s0 = fma_like(a[i], b[i], s0); s1 = fma_like(a[i+1], b[i+1], s1); }
    return s0 + s1;
}

int main(void) {
    /* Results are integral, so printing the raw bits is exact and portable. */
    double acc = 0;
    for (int i = 1; i <= 64; i++) acc += (double)i / (double)(i * i + 1);
    double t = poly(1.0 / 3.0);
    double u = fma_like(1.0 / 7.0, 1.0 / 11.0, 1.0 / 13.0);
    double v = inv(7.0) + inv(11.0) + inv(13.0);
    double w = dpowish(1.0009765625, 10);      /* 2^-10 scaled */
    double x[32], y[32];
    for (int i = 0; i < 32; i++) { x[i] = (double)(i + 1) * 0.03125; y[i] = 1.0 / (double)(i + 3); }
    double m = mix(x, y, 32);

    uint64_t h = 0;
    h ^= (uint64_t)(int64_t)(acc * 4294967296.0);
    h ^= (uint64_t)(int64_t)(t * 1048576.0);
    h ^= (uint64_t)(int64_t)(u * 1048576.0);
    h ^= (uint64_t)(int64_t)(v * 1048576.0);
    h ^= (uint64_t)(int64_t)((w - 1.0) * 67108864.0);
    h ^= (uint64_t)(int64_t)(m * 65536.0);
    float f = fpoly(1.0f / 3.0f);
    uint32_t fb; memcpy(&fb, &f, 4);
    h ^= fb;
    printf("float %016llx\n", (unsigned long long)h);

    /* Conversions in both directions, including the rounding boundary. */
    uint64_t c = 0;
    for (int64_t i = -1000000; i <= 1000000; i += 4999) {
        c = c * 1000003ull + (uint64_t)(int64_t)((double)i / 3.0);
        c += (uint64_t)(uint32_t)((float)i * 0.25f);
    }
    printf("float conv %016llx\n", (unsigned long long)c);
    return 0;
}
