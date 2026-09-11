/*
 * Scalar-FP web corpus: loop shapes that stress the segment-precise FP
 * phi-move check (loop-carried FP webs with fat-overlapping blockers) and
 * the destructive allocator's FP coverage.
 *
 * Every externally-visible noinline function isolates one high-value scalar
 * FP loop shape.  There is deliberately no main(): scripts/godbolt.py and
 * scripts/codegen_oracle.py --rank compare each function against GCC 16.2,
 * Clang, ICC and current ICX without benchmark-harness noise.
 *
 * Run two semantic modes:
 *   strict: -O3 -march=x86-64-v3
 *   fast:   -O3 -march=x86-64-v3 -ffast-math -ffp-contract=fast
 */
#define NOINLINE __attribute__((noinline))

/* 01-04: multi-accumulator dots + polynomial evals. */
NOINLINE double f01_dot2(const double *restrict a, const double *restrict b,
                         int n) {
    double s0 = 0, s1 = 0;
    for (int i = 0; i < n; i += 2) {
        s0 += a[i] * b[i];
        s1 += a[i + 1] * b[i + 1];
    }
    return s0 + s1;
}
NOINLINE double f02_dot4(const double *restrict a, const double *restrict b,
                         int n) {
    double s0 = 0, s1 = 0, s2 = 0, s3 = 0;
    for (int i = 0; i < n; i += 4) {
        s0 += a[i] * b[i];
        s1 += a[i + 1] * b[i + 1];
        s2 += a[i + 2] * b[i + 2];
        s3 += a[i + 3] * b[i + 3];
    }
    return (s0 + s1) + (s2 + s3);
}
NOINLINE double f03_poly_horner(double x, const double *restrict c, int n) {
    double r = c[n - 1];
    for (int i = n - 2; i >= 0; i--) r = r * x + c[i];
    return r;
}
NOINLINE double f04_poly_estrin(double x, const double *restrict c) {
    double x2 = x * x, x4 = x2 * x2;
    double lo = (c[0] + c[1] * x) + (c[2] + c[3] * x) * x2;
    double hi = (c[4] + c[5] * x) + (c[6] + c[7] * x) * x2;
    return lo + hi * x4;
}

/* 05-08: axpy/norm/weighted/fma webs. */
NOINLINE void f05_axpy_f32(float *restrict d, const float *restrict a,
                           float k, int n) {
    for (int i = 0; i < n; i++) d[i] = a[i] * k + d[i];
}
NOINLINE double f06_norm2(const double *restrict a, int n) {
    double s = 0;
    for (int i = 0; i < n; i++) s += a[i] * a[i];
    return s;
}
NOINLINE double f07_wsum(const double *restrict a, const double *restrict w,
                         int n) {
    double s = 0, t = 0;
    for (int i = 0; i < n; i++) {
        s += a[i] * w[i];
        t += w[i];
    }
    return s / t;
}
NOINLINE double f08_fma_chain(const double *restrict a,
                              const double *restrict b,
                              const double *restrict c, int n) {
    double r = 0;
    for (int i = 0; i < n; i++) r = r + a[i] * b[i] + c[i];
    return r;
}

/* 09-12: mixed-precision, vector-sibling, nested, minmax webs. */
NOINLINE double f09_mixed_f32_f64(const float *restrict a,
                                  const double *restrict b, int n) {
    double s = 0;
    for (int i = 0; i < n; i++) s += (double)a[i] * b[i];
    return s;
}
NOINLINE double f10_vec_sibling_sum(const int *restrict a,
                                    const double *restrict b, int n) {
    /* Integer vector accumulator alive across a scalar FP web (p20 shape). */
    long long vi = 0;
    double s = 0;
    for (int i = 0; i < n; i++) {
        vi += a[i];
        s += b[i];
    }
    return s + (double)vi;
}
NOINLINE double f11_nested_fp(const double *restrict a, int n, int m) {
    double t = 0;
    for (int i = 0; i < n; i++) {
        double s = 0;
        for (int j = 0; j < m; j++) s += a[i * m + j];
        t += s * s;
    }
    return t;
}
NOINLINE double f12_minmax_span(const double *restrict a, int n) {
    double mn = a[0], mx = a[0];
    for (int i = 1; i < n; i++) {
        if (a[i] < mn) mn = a[i];
        if (a[i] > mx) mx = a[i];
    }
    return mx - mn;
}

/* 13-15: recurrence, per-element poly+reduce, blocked dot. */
NOINLINE void f13_recurrence(double *restrict a, const double *restrict x,
                             double c, int n) {
    for (int i = 1; i < n; i++) a[i] = a[i - 1] * c + x[i];
}
NOINLINE double f14_poly_reduce(const double *restrict a,
                                const double *restrict c, int n) {
    double t = 0;
    for (int i = 0; i < n; i++) {
        double x = a[i];
        t += ((c[0] * x + c[1]) * x + c[2]) * x + c[3];
    }
    return t;
}
NOINLINE double f15_blocked_dot(const double *restrict a,
                                const double *restrict b, int n) {
    double g = 0;
    for (int i = 0; i < n; i += 8) {
        double s = 0;
        for (int j = 0; j < 8; j++) s += a[i + j] * b[i + j];
        g += s;
    }
    return g;
}
