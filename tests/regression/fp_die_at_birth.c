/*
 * FP producer->consumer register coalescing (die-at-birth
 * hints) for F64/F32 chains, Sqrt/Fabs/Neg, and energy-style accumulation.
 *
 * Every value here is single-use or chain-shaped, so the allocator's
 * die-at-birth sharing kicks in; a miscompile (e.g. the old `subsd
 * %xmm4,%xmm4` ≡ 0) shows up as a wrong number. FP arithmetic is IEEE-exact
 * and expression order is fixed at -O2, so the results are bit-identical to
 * GCC; the harness (--compare-gcc) diffs them.
 */
#include <stdio.h>
#include <math.h>

static double energy(const double *b, int n) {
    double e = 0.0;
    int i;
    for (i = 0; i < n; i++)
        e += 0.5 * b[i] * (b[i] * b[i] + b[i]);
    return e;
}

static double chain64(double x) { return -fabs(sqrt(x) * x + x); }
static float  chain32(float x)  { return -fabsf(sqrtf(x) * x + x); }
static double chain_neg(double x) { return -(-x + x * x); }
static double chain_div(double x) { return (x + 1.0) / (x * x + 1.0); }

static double mixed_sum(const double *a, const double *b, int n) {
    double s = 0.0;
    int i;
    for (i = 0; i < n; i++)
        s += sqrt(a[i] * a[i] + b[i] * b[i]);
    return s;
}

int main(void) {
    double b[32], a[32];
    float bf[32];
    int i, it;
    for (i = 0; i < 32; i++) {
        b[i] = (double)(i + 1) * 0.125;
        a[i] = (double)(32 - i) * 0.0625;
        bf[i] = (float)((i + 1) * 0.125f);
    }

    double e = 0.0;
    for (it = 0; it < 2000; it++) e += energy(b, 32);
    double c = 0.0;
    for (it = 0; it < 2000; it++) c += chain64(1.5 + it * 1e-9);
    float cf = 0.0f;
    for (it = 0; it < 2000; it++) cf += chain32(1.5f + (float)(it * 1e-7));
    double cn = 0.0;
    for (it = 0; it < 2000; it++) cn += chain_neg(0.25 + it * 1e-9);
    double cd = 0.0;
    for (it = 0; it < 2000; it++) cd += chain_div(0.5 + it * 1e-9);
    double ms = mixed_sum(a, b, 32);

    printf("%.9f %.9f %.9f %.9f %.9f %.9f\n", e, c, (double)cf, cn, cd, ms);
    return 0;
}
