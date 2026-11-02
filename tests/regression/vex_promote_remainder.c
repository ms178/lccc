/* vex_promote_remainder — runtime regression for the final VEX-promotion
 * peephole (src/backend/x86/codegen/peephole/passes/vex_promote.rs).
 *
 * Every kernel below vectorises to a VEX-256 loop with a scalar remainder,
 * a scalar reduction epilogue, an int<->float conversion or an FP compare in
 * the same function.  Before the pass those scalar instructions were legacy
 * SSE executed with dirty upper YMM state (merge µop + false dependency on
 * SKL..RPL, ~70-cycle transitions on SNB..BDW).  After the pass they are
 * VEX; bits 127:0 semantics are unchanged, so the outputs must be
 * bit-identical to GCC's (the suite compares stdout) and to the scalar
 * reference computed here with the optimiser locked out via volatile.
 *
 * Trip counts are chosen so that the remainder takes every value 0..7 (and
 * 0..3 for doubles), the sizes where the old tail penalty dominated.
 *
 * The assembly-level property (no legacy SSE in ymm-using functions) is
 * checked by scripts/check_avx_sse_transitions.py --cfg, which this file is
 * also a corpus entry for.
 */
#include <stdio.h>
#include <stdint.h>
#include <string.h>

#define N 71

static float  fa[N + 8], fb[N + 8], fc[N + 8];
static double da[N + 8], db[N + 8], dc[N + 8];
static int    ia[N + 8];

static uint64_t h = 1469598103934665603ull;
static void mix_bytes(const void *p, size_t n)
{
    const unsigned char *b = p;
    for (size_t i = 0; i < n; i++) { h ^= b[i]; h *= 1099511628211ull; }
}
static void mix_f(float x) { mix_bytes(&x, sizeof x); }
static void mix_d(double x) { mix_bytes(&x, sizeof x); }

__attribute__((noinline)) void scale_f(float *restrict y, const float *restrict x, float k, int n)
{ for (int i = 0; i < n; i++) y[i] = x[i] * k; }

__attribute__((noinline)) void axpy_d(double *restrict y, const double *restrict x, double a, int n)
{ for (int i = 0; i < n; i++) y[i] = a * x[i] + y[i]; }

__attribute__((noinline)) float sum_f(const float *x, int n)
{ float s = 0.f; for (int i = 0; i < n; i++) s += x[i]; return s; }

__attribute__((noinline)) double dot_d(const double *x, const double *y, int n)
{ double s = 0.; for (int i = 0; i < n; i++) s += x[i] * y[i]; return s; }

/* int -> float conversion inside the loop: cvtsi2ss in the scalar tail. */
__attribute__((noinline)) void cvt_f(float *restrict y, const int *restrict x, int n)
{ for (int i = 0; i < n; i++) y[i] = (float)x[i] * 0.5f; }

/* float -> int truncation + compare in the tail: cvttss2si / ucomiss. */
__attribute__((noinline)) int count_gt(const float *x, float t, int n)
{ int c = 0; for (int i = 0; i < n; i++) c += x[i] > t; return c; }

__attribute__((noinline)) long trunc_sum(const float *x, int n)
{ long s = 0; for (int i = 0; i < n; i++) s += (long)(x[i] * 3.0f); return s; }

/* max reduction: maxss in the epilogue. */
__attribute__((noinline)) float max_f(const float *x, int n)
{ float m = x[0]; for (int i = 1; i < n; i++) m = x[i] > m ? x[i] : m; return m; }

/* Two kernels in one function: the second loop's scalar tail runs with the
 * first loop's ymm state dirty. */
__attribute__((noinline)) double two_phase(float *restrict f, double *restrict d, int n)
{
    for (int i = 0; i < n; i++) f[i] = f[i] * 2.0f + 1.0f;
    double s = 0.;
    for (int i = 0; i < n; i++) s += d[i] * (double)f[i];
    return s;
}

static void init(int n)
{
    for (int i = 0; i < N + 8; i++) {
        fa[i] = (float)((i * 37) % 101) * 0.25f - 6.f;
        fb[i] = (float)((i * 11) % 53) * 0.5f;
        fc[i] = 0.f;
        da[i] = (double)((i * 29) % 97) * 0.125 - 3.;
        db[i] = (double)((i * 13) % 61) * 0.75;
        dc[i] = 1.0;
        ia[i] = (i * 7919) % 2003 - 1000;
    }
    (void)n;
}

int main(void)
{
    /* Trip counts 1..N: every remainder length for 8-wide and 4-wide loops. */
    for (int n = 1; n <= N; n += (n < 20 ? 1 : 7)) {
        init(n);
        scale_f(fc, fa, 1.5f, n);
        for (int i = 0; i < N + 8; i++) mix_f(fc[i]);
        volatile float kk = 1.5f;
        for (int i = 0; i < n; i++) if (fc[i] != fa[i] * kk) { printf("scale_f mismatch n=%d i=%d\n", n, i); return 1; }

        init(n);
        axpy_d(dc, da, 0.75, n);
        for (int i = 0; i < N + 8; i++) mix_d(dc[i]);

        init(n);
        float s = sum_f(fa, n);
        mix_f(s);
        double dd = dot_d(da, db, n);
        mix_d(dd);

        cvt_f(fc, ia, n);
        for (int i = 0; i < N + 8; i++) mix_f(fc[i]);
        volatile float half = 0.5f;
        for (int i = 0; i < n; i++) if (fc[i] != (float)ia[i] * half) { printf("cvt_f mismatch n=%d i=%d\n", n, i); return 1; }

        int c = count_gt(fa, 2.0f, n);
        long ts = trunc_sum(fa, n);
        float m = max_f(fa, n);
        mix_bytes(&c, sizeof c); mix_bytes(&ts, sizeof ts); mix_f(m);

        init(n);
        double tp = two_phase(fa, da, n);
        mix_d(tp);
        for (int i = 0; i < N + 8; i++) mix_f(fa[i]);

        printf("n=%2d sum=%.9g dot=%.17g gt=%d trunc=%ld max=%.9g two=%.17g\n",
               n, s, dd, c, ts, m, tp);
    }
    printf("hash %016llx\n", (unsigned long long)h);
    return 0;
}
