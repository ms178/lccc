/* Cross-PR interaction red-team battery (session 574).
 *
 * Every shape here stresses an interaction BETWEEN the feature areas of the
 * last ten merged PRs, not the features in isolation (the per-PR gates
 * already do that).  The runtime contract is bit-exact against GCC for
 * everything that contains __builtin_fma (both compilers contract by
 * default and fma is uniquely correctly-rounded, so contracted-vs-
 * contracted is bit-identical by definition) and against
 * gcc -ffp-contract=off for the shapes lccc deliberately leaves
 * uncontracted (the documented residuals).
 */
#include <math.h>
#include <stdio.h>
#include <stdint.h>
#include <string.h>

static const double wtbl[8] = { 1.5, -2.25, 3.125, -4.0625, 5.03125,
                                -6.015625, 7.0078125, -8.00390625 };
static const uint32_t itbl[8] = { 0u, 1u, 0xFFFFFFFFu, 0x80000000u,
                                  0x0000FFFFu, 0xFFFF0000u, 0xA5A5A5A5u,
                                  0x5A5A5A5Au };

/* ---- 1. FMA x vector-copy-elimination: temporaries in copy brackets,
 *         a call inside the bracket, all four families. ---- */
static double bracket_plain(double a, double b, double c) {
    double t = a;                       /* copy-in bracket */
    double r = __builtin_fma(t, b, c);  /* plain family */
    double u = r;                       /* copy-out bracket */
    volatile double sink = u;           /* a use */
    return u + sink - sink;
}
static double bracket_signed(double a, double b, double c) {
    double t = -a;
    double r = __builtin_fma(t, b, -c); /* vfnmsub family */
    double u = r;
    if (u > 1e300) { printf("!"); }     /* branch inside bracket */
    return u;
}
static double bracket_call(double a, double b, double c) {
    double t = a;
    double m = __builtin_fma(t, b, c);
    double u = sqrt(m);                 /* call inside the bracket */
    double v = __builtin_fma(u, b, c);  /* family again after the call */
    return v;
}

/* ---- 2. FMA x if-conversion / range-fusion: select arms over FMA. ---- */
static double sel_fma(double a, double b, double c, int k) {
    /* The compare feeds the select; both arms contract. */
    return k > 0 ? __builtin_fma(a, b, c) : __builtin_fma(a, -b, -c);
}
static double sel_fma_acc(double a, double b, double c, int k) {
    double acc = c;
    if (k & 1) acc = __builtin_fma(a, b, acc);
    if (k & 2) acc = __builtin_fma(-a, b, acc);
    return acc;
}

/* ---- 3. FMA x phi-diamond hoisting: hoistable register inits + fma
 *         arms on both sides. ---- */
static double phi_fma(double a, double b, double c, int k) {
    double hi = 1.5;                    /* hoistable immediate init */
    double lo = a;                      /* hoistable register init */
    if (k) { hi = __builtin_fma(hi, b, c); lo = __builtin_fma(lo, b, c); }
    else   { hi = __builtin_fma(hi, -b, -c); lo = __builtin_fma(lo, -b, -c); }
    return hi * 3.0 + lo;
}

/* ---- 4. FMA x constant-array promotion: weighted dot with the table
 *         in .rodata (the promotion pass must not break the contraction
 *         or the values). ---- */
static double const_dot(const double *x, int n) {
    double s = 0.0;
    for (int i = 0; i < n && i < 8; i++)
        s = __builtin_fma(wtbl[i], x[i], s);
    return s;
}
static double const_dot_map(const double *x, double *r, double bias, int n) {
    double s = 0.0;
    for (int i = 0; i < n && i < 8; i++) {
        r[i] = __builtin_fma(wtbl[i], x[i], bias);   /* map family */
        s += r[i];
    }
    return s;
}

/* ---- 5. NonZero x constant-array promotion: guarded clz/ctz over the
 *         promoted table (the guard-select collapse must still reject
 *         the NonZero variants — values 0 and 1 are in the table). ---- */
static uint32_t nz_scan(uint32_t k) {
    uint32_t acc = 0;
    for (int i = 0; i < 8; i++) {
        uint32_t v = itbl[i] ^ k;
        if (v) acc += (uint32_t)__builtin_clz(v);
    }
    return acc;
}
static uint32_t nz_ctz(uint32_t k) {
    uint32_t acc = 0;
    for (int i = 0; i < 8; i++) {
        uint32_t v = itbl[i] + k;
        acc += v ? (uint32_t)__builtin_ctz(v) : 32u;
    }
    return acc;
}

/* ---- 6. FMA aliased operands (documented residual: ordinary path;
 *         must be CORRECT — bit-exact vs gcc since both contract). ---- */
static double alias_xx(double x, double c) { return __builtin_fma(x, x, c); }
static double alias_ss(double x, double s) { return __builtin_fma(x, s, s); }
static void alias_loop(const double *x, double *r, int n) {
    for (int i = 0; i < n; i++) r[i] = __builtin_fma(x[i], x[i], x[i]);
}

/* ---- 7. FMA multi-use negation (peel grammar: the absorbability
 *         fixpoint) ---- */
static double neg_twouse(double a, double b, double c) {
    double n = -a;
    double r1 = __builtin_fma(n, b, c);
    double r2 = __builtin_fma(n, b, -c);
    return r1 - r2;
}
static double neg_int_reject(int a, double b, double c) {
    /* integer Neg in the product position must NOT peel */
    return __builtin_fma((double)(-a), b, c);
}
static double neg_chain_live_outer(double x, double b, double c) {
    /* The chain defect's own shape: the fma reads the INNER negation while
     * the OUTER negation survives for the add. NEITHER may peel (the outer
     * is pinned by the add; the inner is the outer's operand) -- the
     * source-poisoned rule deleted the inner underneath the outer's
     * surviving read and reached codegen as an ICE. Bit-exact vs gcc,
     * which materialises both links for this shape too. */
    double t = -x;
    double u = -t;
    double r = __builtin_fma(t, b, c);
    return r + u;
}
static double neg_chain_pinned_inner(double x, double b, double c) {
    /* The fold the source-poisoned rule missed: the INNER negation is
     * pinned by the add, the site reads the OUTER. The outer peels into
     * the family reading the materialised inner -- one sign mask, not two
     * (vfnmadd + vxorpd + vaddsd, GCC's exact shape). */
    double t = -x;
    double r = __builtin_fma(-t, b, c);
    return r + t;
}
static double neg_chain_full(double x, double b, double c) {
    /* Both links die: the site peels the whole chain and lands on the
     * PLAIN family reading x (two flips cancel). */
    double t = -x;
    double u = -t;
    return __builtin_fma(u, b, c);
}

/* ---- 8. Wide SLP FMA packs (the P0 width fix: F64x4 / F32x8 must go
 *         through the 256-bit families) mixed with Forward splats. ---- */
static void wide_f64(const double *a, const double *b, double s,
                     double *r) {
    double t0 = __builtin_fma(a[0], s, b[0]);
    double t1 = __builtin_fma(a[1], s, b[1]);
    double t2 = __builtin_fma(a[2], s, b[2]);
    double t3 = __builtin_fma(a[3], s, b[3]);
    r[0] = t0; r[1] = t1; r[2] = t2; r[3] = t3;
}
static void wide_f32(const float *a, const float *b, float s, float *r) {
    float t0 = __builtin_fmaf(a[0], s, b[0]);
    float t1 = __builtin_fmaf(a[1], s, b[1]);
    float t2 = __builtin_fmaf(a[2], s, b[2]);
    float t3 = __builtin_fmaf(a[3], s, b[3]);
    float t4 = __builtin_fmaf(a[4], s, b[4]);
    float t5 = __builtin_fmaf(a[5], s, b[5]);
    float t6 = __builtin_fmaf(a[6], s, b[6]);
    float t7 = __builtin_fmaf(a[7], s, b[7]);
    r[0]=t0; r[1]=t1; r[2]=t2; r[3]=t3; r[4]=t4; r[5]=t5; r[6]=t6; r[7]=t7;
}
static void wide_signed_f64(const double *a, const double *b, double s,
                            double *r) {
    r[0] = __builtin_fma(-a[0], s, -b[0]);
    r[1] = __builtin_fma(-a[1], s, -b[1]);
    r[2] = __builtin_fma(-a[2], s, -b[2]);
    r[3] = __builtin_fma(-a[3], s, -b[3]);
}

/* ---- 9. FMA x peephole store machinery: stores that need the
 *         whitespace-invariant matcher (operand spacing varies with the
 *         emitter path taken). ---- */
static void store_mix(double * restrict r, const double * restrict a,
                      double s, int n) {
    for (int i = 0; i < n; i++)
        r[i] = __builtin_fma(a[i], s, r[i]);   /* in-place acc stream */
}

/* ---- 10. Everything stacked: range fusion + select + copy brackets +
 *          wide packs + table promotion in one function. ---- */
static double kitchen(const double *x, double *r, double bias, int n,
                      int k) {
    double acc = bias;
    for (int i = 0; i < n; i++) {
        double w = (i & 4) ? wtbl[i & 7] : -wtbl[i & 7];
        double v = k > 0 ? __builtin_fma(w, x[i], bias)
                         : __builtin_fma(-w, x[i], -bias);
        r[i] = v;
        acc += v;
    }
    return acc;
}

/* ---------------- driver ---------------- */
static uint64_t fbits(double d) {
    uint64_t u; memcpy(&u, &d, 8); return u;
}
static int same(double a, double b) {
    if (isnan(a) && isnan(b)) return 1;
    return fbits(a) == fbits(b);
}
/* Print-time canonical-NaN normalisation (the C11 6.5p8 latitude, the
 * same one the dedicated gate normalises in its diff): when an fma
 * operand is NaN, WHICH NaN the operation propagates -- and with which
 * sign -- is implementation-defined. The default baseline here differs
 * on purpose from the reference compiler's (house v3 default contracts
 * __builtin_fma into the hardware family, which propagates a source
 * NaN's sign UNNEGATED, CPU-verified; the reference compiler at its own
 * default keeps the libm call, which propagates the first NaN operand's
 * sign). Both are conforming, so the printed oracle canonicalises every
 * NaN on BOTH sides; every other bit -- including -0.0 signs -- stays
 * bit-exact, and the asm pins in check_cross_pr_redteam.sh fix the exact
 * family selection. */
static uint64_t nbits(double d) {
    return isnan(d) ? 0x7ff8000000000000ULL : fbits(d);
}
static uint32_t nbits32(float f) {
    uint32_t u; memcpy(&u, &f, 4);
    return isnan(f) ? 0x7fc00000u : u;
}

int main(void) {
    static const double xs[] = { 0.0, -0.0, 1.0, -1.0, 0.5, INFINITY,
                                 -INFINITY, NAN, 1e308, 1e-308, 3.14159,
                                 -2.71828, 65536.0, 1e-300, -1e300, 42.0 };
    static const float  fs[] = { 0.0f, -0.0f, 1.0f, -1.0f, 0.5f, INFINITY,
                                 -INFINITY, NAN, 1e30f, 1e-30f, 3.14f };
    enum { NX = (int)(sizeof xs / sizeof xs[0]),
           NF = (int)(sizeof fs / sizeof fs[0]) };
    double r8[8], xbuf[700], big[256];
    int fails = 0;

    /* 1 */ for (int i = 0; i < NX; i++) for (int j = 0; j < NX; j += 3)
        for (int m = 0; m < NX; m += 5) {
            double g1 = bracket_plain(xs[i], xs[j], xs[m]);
            double g2 = bracket_signed(xs[i], xs[j], xs[m]);
            double g3 = bracket_call(xs[i], fabs(xs[j]) + 1, xs[m]);
            printf("A %d %d %d %016llx %016llx %016llx\n", i, j, m,
                   (unsigned long long)nbits(g1),
                   (unsigned long long)nbits(g2),
                   (unsigned long long)nbits(g3));
        }
    /* 2 */ for (int i = 0; i < NX; i++) for (int j = 0; j < NX; j += 3)
        for (int m = 0; m < NX; m += 5) for (int k = 0; k < 4; k++) {
            double g1 = sel_fma(xs[i], xs[j], xs[m], k);
            double g2 = sel_fma_acc(xs[i], xs[j], xs[m], k);
            printf("B %d %d %d %d %016llx %016llx\n", i, j, m, k,
                   (unsigned long long)nbits(g1),
                   (unsigned long long)nbits(g2));
        }
    /* 3 */ for (int i = 0; i < NX; i++) for (int j = 0; j < NX; j += 7)
        for (int m = 0; m < NX; m += 3) for (int k = 0; k < 2; k++) {
            double g = phi_fma(xs[i], xs[j], xs[m], k);
            printf("C %d %d %d %d %016llx\n", i, j, m, k,
                   (unsigned long long)nbits(g));
        }
    /* 4 */ for (int n = 0; n <= 8; n++) {
        for (int i = 0; i < 8; i++) xbuf[i] = xs[(i * 3) % NX];
        double g1 = const_dot(xbuf, n);
        double g2 = const_dot_map(xbuf, r8, xs[5], n);
        printf("D %d %016llx %016llx\n", n,
               (unsigned long long)nbits(g1),
               (unsigned long long)nbits(g2));
        for (int i = 0; i < n && i < 8; i++)
            printf("d %d %016llx\n", i, (unsigned long long)nbits(r8[i]));
    }
    /* 5 */ for (uint32_t k = 0; k < 8; k++)
        printf("E %u %u %u\n", k, nz_scan(k), nz_ctz(k));
    /* 6 */ for (int i = 0; i < NX; i++) {
        double g1 = alias_xx(xs[i], xs[(i * 5) % NX]);
        double g2 = alias_ss(xs[i], xs[(i * 7) % NX]);
        printf("F %d %016llx %016llx\n", i,
               (unsigned long long)nbits(g1),
               (unsigned long long)nbits(g2));
    }
    for (int n = 0; n < 64; n += 7) {
        for (int i = 0; i < n; i++) xbuf[i] = xs[i % NX];
        alias_loop(xbuf, big, n);
        for (int i = 0; i < n; i++)
            printf("f %d %d %016llx\n", n, i,
                   (unsigned long long)nbits(big[i]));
    }
    /* 7 */ for (int i = 0; i < NX; i++) for (int j = 0; j < NX; j += 3)
        for (int m = 0; m < NX; m += 5) {
            double g1 = neg_twouse(xs[i], xs[j], xs[m]);
            double g2 = neg_int_reject((int)(i - 8), xs[j], xs[m]);
            double g3 = neg_chain_live_outer(xs[i], xs[j], xs[m]);
            double g4 = neg_chain_pinned_inner(xs[i], xs[j], xs[m]);
            double g5 = neg_chain_full(xs[i], xs[j], xs[m]);
            printf("G %d %d %d %016llx %016llx %016llx %016llx %016llx\n",
                   i, j, m,
                   (unsigned long long)nbits(g1),
                   (unsigned long long)nbits(g2),
                   (unsigned long long)nbits(g3),
                   (unsigned long long)nbits(g4),
                   (unsigned long long)nbits(g5));
        }
    /* 8 */ for (int i = 0; i + 7 < NX; i += 2) {
        double ad[4], bd[4]; float af[8], bf[8];
        for (int t = 0; t < 4; t++) { ad[t] = xs[i + t]; bd[t] = xs[i + t + 1]; }
        for (int t = 0; t < 8; t++) { af[t] = fs[(i + t) % NF]; bf[t] = fs[(i + t + 3) % NF]; }
        wide_f64(ad, bd, xs[4], r8);
        printf("H %d %016llx %016llx %016llx %016llx\n", i,
               (unsigned long long)nbits(r8[0]), (unsigned long long)nbits(r8[1]),
               (unsigned long long)nbits(r8[2]), (unsigned long long)nbits(r8[3]));
        wide_signed_f64(ad, bd, xs[6], r8);
        printf("h %d %016llx %016llx %016llx %016llx\n", i,
               (unsigned long long)nbits(r8[0]), (unsigned long long)nbits(r8[1]),
               (unsigned long long)nbits(r8[2]), (unsigned long long)nbits(r8[3]));
        float rf[8]; wide_f32(af, bf, fs[5], rf);
        printf("I %d %08x %08x %08x %08x %08x %08x %08x %08x\n", i,
               nbits32(rf[0]), nbits32(rf[1]),
               nbits32(rf[2]), nbits32(rf[3]),
               nbits32(rf[4]), nbits32(rf[5]),
               nbits32(rf[6]), nbits32(rf[7]));
    }
    /* 9 */ for (int n = 0; n < 128; n += 13) {
        for (int i = 0; i < n; i++) xbuf[i] = (i & 1) ? xs[i % NX] : -xs[i % NX];
        store_mix(xbuf, xbuf, xs[2], n);
        for (int i = 0; i < n; i += 5)
            printf("J %d %d %016llx\n", n, i, (unsigned long long)nbits(xbuf[i]));
    }
    /* 10 */ for (int n = 0; n < 200; n += 17) for (int k = 0; k < 2; k++) {
        for (int i = 0; i < n; i++) xbuf[i] = xs[i % NX];
        double g = kitchen(xbuf, big, xs[3], n, k);
        printf("K %d %d %016llx\n", n, k, (unsigned long long)nbits(g));
        for (int i = 0; i < n && i < 8; i++)
            printf("k %d %d %016llx\n", n, i, (unsigned long long)nbits(big[i]));
    }
    printf("FAILS %d\n", fails);
    (void)same; (void)fails;
    return 0;
}
