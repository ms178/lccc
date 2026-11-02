/* A select whose condition's flags are still live (fused integer Cmp, the
 * FP-select fallback's ucomis, or a replayed compare) never materialized
 * the boolean, so it MUST consume the flags in place. CCC_NO_INPLACE_SELECT
 * (this test's .env) used to override that and send the select down the
 * materialized path, which tested a stale home: at -O2/-O3 every select
 * below could take the wrong arm. The kill switch now only disables the
 * optional in-place test of an already-materialized condition.
 *
 * Self-checking (the reference keeps the comparison result in a volatile,
 * so its select tests a materialized value) and stdout-compared with GCC. */
#include <math.h>
#include <stdio.h>

__attribute__((noinline)) int sel_lt_i(int a, int b, int x, int y) { return a < b ? x : y; }
__attribute__((noinline)) long sel_gtu_l(unsigned long a, unsigned long b, long x, long y) {
    return a > b ? x : y;
}
__attribute__((noinline)) int sel_lt_d(double a, double b, int x, int y) { return a < b ? x : y; }
__attribute__((noinline)) int sel_le_d(double a, double b, int x, int y) { return a <= b ? x : y; }
__attribute__((noinline)) int sel_gt_f(float a, float b, int x, int y) { return a > b ? x : y; }
__attribute__((noinline)) int sel_ge_f(float a, float b, int x, int y) { return a >= b ? x : y; }
__attribute__((noinline)) int sel_eq_d(double a, double b, int x, int y) { return a == b ? x : y; }
__attribute__((noinline)) int sel_ne_d(double a, double b, int x, int y) { return a != b ? x : y; }
/* The compare's flags are clobbered by the multiply before the select. */
__attribute__((noinline)) int sel_replay(int a, int b, int x, int y) {
    int c = a < b;
    int m = x * y;
    return (c ? x : y) + m;
}

static int ref(int c, int x, int y) {
    volatile int v = c;
    if (v)
        return x;
    return y;
}

int main(void) {
    static const double d[] = {-1.5, 0.0, -0.0, 2.25, INFINITY, NAN};
    static const int iv[] = {-3, -1, 0, 1, 2147483647, -2147483647 - 1};
    unsigned long h = 0;
    int bad = 0;
#define CHECK(got, want)                                                                 \
    do {                                                                                 \
        long g_ = (got), w_ = (want);                                                    \
        if (g_ != w_) {                                                                  \
            printf("FAIL line %d: %ld != %ld\n", __LINE__, g_, w_);                      \
            bad = 1;                                                                     \
        }                                                                                \
        h = h * 131u + (unsigned long)g_;                                                \
    } while (0)
    for (int i = 0; i < 6; i++) {
        for (int j = 0; j < 6; j++) {
            double a = d[i], b = d[j];
            CHECK(sel_lt_d(a, b, 7, 11), ref(a < b, 7, 11));
            CHECK(sel_le_d(a, b, 7, 11), ref(a <= b, 7, 11));
            CHECK(sel_gt_f((float)a, (float)b, 5, 13), ref((float)a > (float)b, 5, 13));
            CHECK(sel_ge_f((float)a, (float)b, 5, 13), ref((float)a >= (float)b, 5, 13));
            CHECK(sel_eq_d(a, b, 3, 17), ref(a == b, 3, 17));
            CHECK(sel_ne_d(a, b, 3, 17), ref(a != b, 3, 17));
            int p = iv[i], q = iv[j];
            CHECK(sel_lt_i(p, q, 19, 23), ref(p < q, 19, 23));
            CHECK(sel_gtu_l((unsigned long)p, (unsigned long)q, 29, 31),
                  ref((unsigned long)p > (unsigned long)q, 29, 31));
            CHECK(sel_replay(p, q, 37, 41), ref(p < q, 37, 41) + 37 * 41);
        }
    }
    printf("hash %lu\n", h);
    return bad;
}
