/* Legal one-source copy/scale/add/affine loops must preserve exact scalar
 * semantics across vector-width boundaries.  This covers F32/F64/I32, I64
 * induction, exact in-place maps, signed negative bounds, and an overlapping
 * non-restrict loop that must remain scalar. */
#include <stdio.h>

#define CAP 96

__attribute__((noinline))
static void copy_f64(double *restrict d, const double *restrict a, int n) {
    for (int i = 0; i < n; ++i) d[i] = a[i];
}

__attribute__((noinline))
static void scale_f32(float *restrict d, const float *restrict a, float scale,
                      int n) {
    for (int i = 0; i < n; ++i) d[i] = a[i] * scale;
}

__attribute__((noinline))
static void add_f32(float *restrict d, const float *restrict a, float bias,
                    unsigned n) {
    for (unsigned i = 0; i < n; ++i) d[i] = a[i] + bias;
}

__attribute__((noinline))
static void affine_f64(double *restrict d, const double *restrict a,
                       double scale, double bias, int n) {
    for (int i = 0; i < n; ++i) d[i] = a[i] * scale + bias;
}

__attribute__((noinline))
static void affine_i32(int *restrict d, const int *restrict a,
                       int scale, int bias, int n) {
    for (int i = 0; i < n; ++i) d[i] = a[i] * scale + bias;
}

__attribute__((noinline))
static void affine_i64_iv(float *restrict d, const float *restrict a,
                          float scale, float bias, long n) {
    for (long i = 0; i < n; ++i) d[i] = a[i] * scale + bias;
}

__attribute__((noinline))
static void in_place_f64(double *a, double scale, double bias, int n) {
    for (int i = 0; i < n; ++i) a[i] = a[i] * scale + bias;
}

/* d == a + 1 creates a true loop-carried dependence.  Different GEP SSA
 * values with the same object root are not an alias/disjointness proof. */
__attribute__((noinline))
static void shifted_overlap_f64(double *d, const double *a, int n) {
    for (int i = 0; i < n; ++i) d[i] = a[i] * 2.0 + 1.0;
}

static void init(double *sd, double *dd, double *rd,
                 float *sf, float *df, float *rf,
                 int *si, int *di, int *ri) {
    for (int i = 0; i < CAP; ++i) {
        sd[i] = (double)(i - 31);
        dd[i] = rd[i] = -9001.0;
        sf[i] = (float)(i - 31);
        df[i] = rf[i] = -9001.0f;
        si[i] = i - 31;
        di[i] = ri[i] = -9001;
    }
}

static int check_d(const char *name, const double *got, const double *want,
                   int n) {
    for (int i = 0; i < CAP; ++i) {
        if (got[i] != want[i]) {
            printf("FAIL %s n=%d i=%d got=%.0f want=%.0f\n",
                   name, n, i, got[i], want[i]);
            return 1;
        }
    }
    return 0;
}

static int check_f(const char *name, const float *got, const float *want,
                   int n) {
    for (int i = 0; i < CAP; ++i) {
        if (got[i] != want[i]) {
            printf("FAIL %s n=%d i=%d got=%.0f want=%.0f\n",
                   name, n, i, (double)got[i], (double)want[i]);
            return 1;
        }
    }
    return 0;
}

static int check_i(const char *name, const int *got, const int *want, int n) {
    for (int i = 0; i < CAP; ++i) {
        if (got[i] != want[i]) {
            printf("FAIL %s n=%d i=%d got=%d want=%d\n",
                   name, n, i, got[i], want[i]);
            return 1;
        }
    }
    return 0;
}

int main(void) {
    static double sd[CAP], dd[CAP], rd[CAP];
    static float sf[CAP], df[CAP], rf[CAP];
    static int si[CAP], di[CAP], ri[CAP];
    static const int bounds[] = {
        -9, -1, 0, 1, 2, 3, 4, 5, 7, 8, 9, 15, 16, 17,
        31, 32, 33, 63, 64, 65, 79
    };

    for (unsigned k = 0; k < sizeof(bounds) / sizeof(bounds[0]); ++k) {
        int n = bounds[k];
        int positive_n = n > 0 ? n : 0;

        init(sd, dd, rd, sf, df, rf, si, di, ri);
        copy_f64(dd, sd, n);
        for (int i = 0; i < positive_n; ++i) rd[i] = sd[i];
        if (check_d("copy_f64", dd, rd, n)) return 1;

        init(sd, dd, rd, sf, df, rf, si, di, ri);
        scale_f32(df, sf, 2.0f, n);
        for (int i = 0; i < positive_n; ++i) rf[i] = sf[i] * 2.0f;
        if (check_f("scale_f32", df, rf, n)) return 1;

        if (n >= 0) {
            init(sd, dd, rd, sf, df, rf, si, di, ri);
            add_f32(df, sf, 5.0f, (unsigned)n);
            for (int i = 0; i < n; ++i) rf[i] = sf[i] + 5.0f;
            if (check_f("add_f32", df, rf, n)) return 1;
        }

        init(sd, dd, rd, sf, df, rf, si, di, ri);
        affine_f64(dd, sd, 3.0, -2.0, n);
        for (int i = 0; i < positive_n; ++i) rd[i] = sd[i] * 3.0 - 2.0;
        if (check_d("affine_f64", dd, rd, n)) return 1;

        init(sd, dd, rd, sf, df, rf, si, di, ri);
        affine_i32(di, si, 3, -2, n);
        for (int i = 0; i < positive_n; ++i) ri[i] = si[i] * 3 - 2;
        if (check_i("affine_i32", di, ri, n)) return 1;

        init(sd, dd, rd, sf, df, rf, si, di, ri);
        affine_i64_iv(df, sf, 3.0f, -2.0f, (long)n);
        for (int i = 0; i < positive_n; ++i) rf[i] = sf[i] * 3.0f - 2.0f;
        if (check_f("affine_i64_iv", df, rf, n)) return 1;

        init(sd, dd, rd, sf, df, rf, si, di, ri);
        for (int i = 0; i < CAP; ++i) dd[i] = rd[i] = sd[i];
        in_place_f64(dd, 2.0, 1.0, n);
        for (int i = 0; i < positive_n; ++i) rd[i] = rd[i] * 2.0 + 1.0;
        if (check_d("in_place_f64", dd, rd, n)) return 1;

        init(sd, dd, rd, sf, df, rf, si, di, ri);
        for (int i = 0; i < CAP; ++i) dd[i] = rd[i] = sd[i];
        shifted_overlap_f64(dd + 1, dd, n);
        for (int i = 0; i < positive_n; ++i) rd[i + 1] = rd[i] * 2.0 + 1.0;
        if (check_d("shifted_overlap_f64", dd, rd, n)) return 1;
    }

    puts("OK affine map vectorization");
    return 0;
}
