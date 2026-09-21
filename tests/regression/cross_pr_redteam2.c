/* PR-area adversarial battery 2 (session 574): range-check fusion (#569),
 * constant-array promotion (#566), vector copy elimination (#567) x the
 * new multi-use FMA peel, and register-pressure shapes (#569 regalloc).
 * Runtime contract: bit-exact against GCC -O2 -march=x86-64-v3. */
#include <math.h>
#include <stdio.h>
#include <stdint.h>
#include <string.h>

/* ---- 1. range fusion corners (integer; the fused domain check) ---- */
static int rf_int(int x) {
    if (x >= -3 && x <= 7) return 1;
    return 0;
}
static int rf_empty(int x) {          /* lo > hi: empty domain */
    if (x >= 7 && x <= -3) return 1;
    return 0;
}
static int rf_point(int x) {          /* lo == hi */
    if (x >= 5 && x <= 5) return 1;
    return 0;
}
static int rf_int_min(int x) {        /* INT_MIN corner */
    if (x >= (-2147483647 - 1) && x <= 0) return 1;
    return 0;
}
static int rf_or(unsigned x) {        /* disjunction shape */
    if (x < 10 || x > 100) return 1;
    return 0;
}
static unsigned rf_or_wrap(void) {    /* u > 0xFFFFFF00 || u < 0x10 */
    unsigned acc = 0;
    for (unsigned u = 0xFFFFFFF0u; u != 0x20; u += 3)
        if (u > 0xFFFFFF00u || u < 0x10u) acc++;
    return acc;
}

/* ---- 2. FP range shapes: must NOT break NaN semantics ---- */
static int rf_fp(double d) {
    if (d >= 0.0 && d <= 1.0) return 1;   /* NaN: both false -> 0 */
    return 0;
}
static int rf_fp_nan_branch(double d) {
    if (d > 0.0 && d < 1.0) return 2;     /* NaN: 0 */
    if (d >= 0.0 && d <= 1.0) return 1;
    return 0;
}
static int rf_fp_reversed(double d) {     /* reversed operands */
    if (0.0 <= d && 1.0 >= d) return 1;
    return 0;
}

/* ---- 3. constant-array promotion x aliasing ---- */
static const int itab[8] = { 1, 2, 4, 8, 16, 32, 64, 128 };
static const double dtab[6] = { 0.5, 1.25, 2.125, 4.0625, 8.03125, 16.015625 };
static int promo_alias(int *p, unsigned k) {
    /* The table is const, but p may alias it in the caller's world (cast
     * away const). The promoted form must observe the WRITE. */
    int s = 0;
    for (int i = 0; i < 8; i++) s += itab[i];
    *p = 999;
    for (int i = 0; i < 8; i++) s += itab[(k + (unsigned)i) & 7u];
    return s;
}
static double promo_dtab(unsigned k) {
    double s = 0.0;
    for (int i = 0; i < 6; i++)
        s = __builtin_fma(dtab[i], (double)(int)((k >> i) & 1u), s);
    return s;
}

/* ---- 4. copy-elim x multi-use FMA peel interaction: the fma results
 *         through copy brackets, packed reads, and a call. ---- */
static double ce_fma(double a, double b, double c) {
    double t = a;                             /* copy-in */
    double r1 = __builtin_fma(-t, b, -c);     /* multi-use -t? single here */
    double u = __builtin_fma(-a, b, -c);      /* second site sharing -a/-c
                                                 spelling at the source level */
    double v = r1;                            /* copy-out */
    if (v < -1e300) printf("!");
    return v + u;
}
static double ce_fma_call(double a, double b, double c) {
    double r = __builtin_fma(-a, b, -c);
    double s = sqrt(fabs(r));                 /* call inside the bracket */
    return __builtin_fma(-s, b, -c);
}

/* ---- 5. regalloc pressure: many live FMA accumulators (web stress) ---- */
static double pressure_fma(const double *x, const double *y, int n) {
    double a0=0,a1=0,a2=0,a3=0,a4=0,a5=0,a6=0,a7=0;
    for (int i = 0; i < n; i++) {
        a0 = __builtin_fma(x[i],   y[i], a0);
        a1 = __builtin_fma(-x[i],  y[i], a1);
        a2 = __builtin_fma(x[i],  -y[i], a2);
        a3 = __builtin_fma(-x[i], -y[i], a3);
        a4 = __builtin_fma(x[i+8], y[i], a4);
        a5 = __builtin_fma(-x[i+8],y[i], a5);
        a6 = __builtin_fma(x[i+8],-y[i], a6);
        a7 = __builtin_fma(-x[i+8],-y[i], a7);
    }
    return (((a0*3+a1*5)+a2*7)+a3*11)+(((a4*13+a5*17)+a6*19)+a7*23);
}

/* ---- 6. mixed-width pressure: f32 and f64 fma webs in one function ---- */
static double mixed_width(const float *xf, const double *xd, int n) {
    float  f0 = 0.f, f1 = 0.f;
    double d0 = 0.0, d1 = 0.0;
    for (int i = 0; i < n; i++) {
        f0 = __builtin_fmaf(-xf[i], xf[i+n], f0);
        f1 = __builtin_fmaf(xf[i], -xf[i+n], f1);
        d0 = __builtin_fma(-xd[i], xd[i+n], d0);
        d1 = __builtin_fma(xd[i], -xd[i+n], d1);
    }
    return (double)(f0 - f1) + (d0 - d1);
}

/* ---------------- driver ---------------- */
static uint64_t fbits(double d) { uint64_t u; memcpy(&u, &d, 8); return u; }
int main(void) {
    static const int iv[] = { -2147483647-1, -2147483647, -8, -3, -2, -1, 0,
                              1, 4, 5, 6, 7, 8, 100, 2147483647 };
    static const double dv[] = { -1.0, -0.0, 0.0, 0.5, 0.999, 1.0, 1.001,
                                 INFINITY, -INFINITY, NAN, -NAN, 1e308,
                                 1e-308, 3.5 };
    static const float fv[] = { -1.0f, 0.0f, 0.5f, 1.0f, 2.5f, -0.5f, 1e30f,
                                -1e30f, NAN };
    double xbuf[512], ybuf[512]; float xfbuf[512];
    for (int i = 0; i < 512; i++) {
        xbuf[i] = dv[i % 14] * (i & 1 ? -1.0 : 1.0);
        ybuf[i] = dv[(i * 5) % 14];
        xfbuf[i] = fv[i % 9];
    }
    for (int i = 0; i < 15; i++)
        printf("R %d %d %d %d %d\n", iv[i], rf_int(iv[i]), rf_empty(iv[i]),
               rf_point(iv[i]), rf_int_min(iv[i]));
    for (unsigned u = 0; u < 200; u += 17)
        printf("O %u %d\n", u, rf_or(u));
    printf("W %u\n", rf_or_wrap());
    for (int i = 0; i < 14; i++)
        printf("F %d %d %d %d %016llx\n", i, rf_fp(dv[i]),
               rf_fp_nan_branch(dv[i]), rf_fp_reversed(dv[i]),
               (unsigned long long)fbits(dv[i]));
    { int q = 0; printf("P %d\n", promo_alias(&q, 5)); }
    for (unsigned k = 0; k < 16; k++)
        printf("Q %u %016llx\n", k, (unsigned long long)fbits(promo_dtab(k)));
    for (int i = 0; i < 14; i++) for (int j = 0; j < 14; j += 3)
        printf("C %d %d %016llx %016llx\n", i, j,
               (unsigned long long)fbits(ce_fma(dv[i], dv[j], dv[(i+3)%14])),
               (unsigned long long)fbits(ce_fma_call(dv[i], fabs(dv[j]) + 1.0, dv[(i+5)%14])));
    for (int n = 0; n <= 128; n += 16)
        printf("S %d %016llx\n", n, (unsigned long long)fbits(pressure_fma(xbuf, ybuf, n)));
    for (int n = 0; n <= 128; n += 32)
        printf("M %d %016llx\n", n, (unsigned long long)fbits(mixed_width(xfbuf, xbuf, n)));
    return 0;
}
