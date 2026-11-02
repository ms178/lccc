/*
 * OP-05a: elementwise map expression trees (sqrt/div/sub/multi-stream).
 * Every kernel is checked elementwise against a volatile-anchored scalar
 * reference, over a size that exercises both the packed body (130 = 16x8+2)
 * and the scalar remainder.
 */
#include <stdio.h>
#include <math.h>

#define N 130

static double d[N], s[N], a3[N], b3[N], c3[N];
static float fd[N], fs[N];

void sqrt_scale(double *restrict dst, const double *restrict src, int n, double k) {
    for (int i = 0; i < n; i++) dst[i] = sqrt(src[i]) * k;
}
void reciprocal(float *restrict dst, const float *restrict src, int n) {
    for (int i = 0; i < n; i++) dst[i] = 1.0f / src[i];
}
void three_stream(double *restrict dst, const double *restrict a,
                  const double *restrict b, const double *restrict c, int n) {
    for (int i = 0; i < n; i++) dst[i] = a[i] * b[i] + c[i];
}
void mixed(float *restrict dst, const float *restrict src, int n) {
    for (int i = 0; i < n; i++)
        dst[i] = sqrtf(src[i] * src[i] + 1.0f) - src[i] / 3.0f;
}

int main(void) {
    for (int i = 0; i < N; i++) {
        s[i] = 0.25 + i * 0.01;
        fs[i] = 0.5f + i * 0.02f;
        a3[i] = i + 1.0;
        b3[i] = 2.0 - i * 0.005;
        c3[i] = i * 0.5;
    }

    int bad = 0;

    sqrt_scale(d, s, N, 2.0);
    for (int i = 0; i < N; i++) {
        volatile double x = s[i];
        if (d[i] != sqrt(x) * 2.0) bad++;
    }

    reciprocal(fd, fs, N);
    for (int i = 0; i < N; i++) {
        volatile float x = fs[i];
        if (fd[i] != 1.0f / x) bad++;
    }

    three_stream(d, a3, b3, c3, N);
    for (int i = 0; i < N; i++) {
        volatile double x = a3[i], y = b3[i], z = c3[i];
        if (d[i] != x * y + z) bad++;
    }

    mixed(fd, fs, N);
    for (int i = 0; i < N; i++) {
        volatile float x = fs[i];
        if (fd[i] != sqrtf(x * x + 1.0f) - x / 3.0f) bad++;
    }

    printf("%s\n", bad == 0 ? "OK" : "FAIL");
    return bad == 0 ? 0 : 1;
}
