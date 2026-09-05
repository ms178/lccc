/* Regression: copy-connected FP webs whose members INTERFERE.
 *
 * Every function below is a scalar FP "copy web" after phi elimination
 * (Copy edges connect the values), yet the members hold DIFFERENT values at
 * the same time: Fibonacci/rotation swaps, a reduction result reused after
 * the remainder loop, prev/cur Newton tracking, a snapshot taken mid-loop.
 * A coalescer that unions copy edges without an interference check (fat
 * interval [min start, max end] shared by all members) gives them ONE XMM
 * register and the latch copy of one member destroys the other.  Executed
 * against GCC at -O2/-O3 by the suite; also exercised with
 * -march=x86-64-v3 (AVX reduction + remainder tails) via the .flags sidecar.
 */
#include <stdio.h>
#include <math.h>
/* 1. FP swap web: a and b are copy-connected but interfere. */
__attribute__((noinline)) double fib_d(long n) {
    double a = 0.0, b = 1.0;
    for (long i = 0; i < n; i++) { double t = a + b; a = b; b = t; }
    return a;
}
__attribute__((noinline)) float fib_f(long n) {
    float a = 0.0f, b = 1.0f;
    for (long i = 0; i < n; i++) { float t = a + b; a = b; b = t; }
    return a;
}
/* 2. Reduction combine value reused after the remainder loop. */
__attribute__((noinline)) double red_reuse(const double *x, int n, int m) {
    double s = 0.0;
    for (int i = 0; i < n; i++) s += x[i];
    double t = s;
    for (int i = 0; i < m; i++) t += x[i] * 0.5;
    return s - t;
}
__attribute__((noinline)) float red_reuse_f(const float *x, int n, int m) {
    float s = 0.0f;
    for (int i = 0; i < n; i++) s += x[i];
    float t = s;
    for (int i = 0; i < m; i++) t += x[i] * 0.5f;
    return s - t;
}
/* 3. Newton with prev tracking (prev/cur copy web, both live). */
__attribute__((noinline)) double newton(double a, int iters) {
    double cur = a, prev = 0.0;
    for (int i = 0; i < iters; i++) { prev = cur; cur = 0.5 * (cur + a / cur); }
    return cur - prev;
}
/* 4. Lost-copy: value copied, then original modified, both used. */
__attribute__((noinline)) double lost_copy(const double *x, int n) {
    double acc = 0.0, snap = 0.0;
    for (int i = 0; i < n; i++) {
        if (i % 7 == 3) snap = acc;   /* copy of acc while acc keeps changing */
        acc += x[i];
    }
    return acc * 3.0 + snap;
}
/* 5. Three-way rotation web. */
__attribute__((noinline)) double rot3(const double *x, int n) {
    double a = 1.0, b = 2.0, c = 3.0;
    for (int i = 0; i < n; i++) { double t = a; a = b + x[i]; b = c; c = t; }
    return a * 1.0 + b * 10.0 + c * 100.0;
}
/* 6. Vector reduction + scalar remainder where the combine is also returned. */
__attribute__((noinline)) double dot_and_sum(const double *x, const double *y, int n, double *sum_out) {
    double d = 0.0;
    for (int i = 0; i < n; i++) d += x[i] * y[i];
    double s = d;
    for (int i = 0; i < n; i++) s += y[i];
    *sum_out = d;
    return s;
}
int main(void) {
    static double x[257], xs[257]; static float xf[257];
    for (int i = 0; i < 257; i++) { x[i] = (i % 13) * 0.25 - 1.5; xf[i] = (float)x[i]; xs[i] = 1.0 / (i + 1); }
    printf("fib_d %.1f\n", fib_d(30));
    printf("fib_f %.1f\n", fib_f(20));
    printf("red_reuse %.6f %.6f %.6f\n", red_reuse(x, 257, 7), red_reuse(x, 16, 16), red_reuse(x, 3, 100));
    printf("red_reuse_f %.6f %.6f\n", red_reuse_f(xf, 257, 7), red_reuse_f(xf, 33, 5));
    printf("newton %.9f %.9f\n", newton(2.0, 6), newton(9.0, 1));
    printf("lost_copy %.6f %.6f\n", lost_copy(x, 257), lost_copy(x, 4));
    printf("rot3 %.6f %.6f\n", rot3(x, 257), rot3(x, 5));
    double so; double r = dot_and_sum(x, xs, 257, &so);
    printf("dot_and_sum %.6f %.6f\n", r, so);
    return 0;
}
