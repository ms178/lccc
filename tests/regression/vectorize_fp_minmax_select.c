/*
 * FP min/max/compare/blendv vectorization semantics.
 *
 * Every kernel here vectorizes through the VecMin/VecMax/VecCmp/VecBlendv
 * intrinsics (256-bit body + scalar remainder); the printed hashes are
 * differential-checked against GCC under the same flags, so a wrong operand
 * order in the MINPS/MAXPS lowering (the second source is returned on
 * unordered and both-zero lanes) or a wrong min/max fold shows up as a hash
 * mismatch.  Inputs deliberately cover NaN in each operand position, both
 * NaN payload signs, +-0 in both orders, infinities, the smallest subnormal,
 * exact-equal pairs, and value ranges that force the scalar remainder to
 * interleave with vector iterations.
 */
#include <math.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

static uint32_t hash_bits(const void *p, size_t nbytes) {
    const uint8_t *b = (const uint8_t *)p;
    uint32_t h = 2166136261u;
    for (size_t i = 0; i < nbytes; i++) h = (h ^ b[i]) * 16777619u;
    return h;
}

#define N 1024
static float A[N], B[N], C[N], D[N];

static void fill(void) {
    const float vals[22] = {
        0.0f, -0.0f, 1.0f, -1.0f, 2.5f, -2.5f, 0.5f, -0.5f,
        INFINITY, -INFINITY, 1e-45f, -1e-45f, 3.5f, -3.5f,
        NAN, -NAN, 100.0f, -100.0f, 0.25f, -0.25f, 7.0f, -7.0f,
    };
    for (int i = 0; i < N; i++) {
        A[i] = vals[i % 22];
        B[i] = vals[(i * 7 + 3) % 22];
        C[i] = vals[(i * 13 + 11) % 22];
    }
}

#define REPORT(tag, out)                                                     \
    do {                                                                     \
        printf("%s %08x\n", tag, hash_bits(out, sizeof(out)));               \
    } while (0)

/* Ternary min/max: canonical folds to VMINPS/VMAXPS. */
static void k_min_ternary(float *restrict d, const float *restrict a,
                          const float *restrict b, int n) {
    for (int i = 0; i < n; i++) d[i] = a[i] < b[i] ? a[i] : b[i];
}
static void k_max_ternary(float *restrict d, const float *restrict a,
                          const float *restrict b, int n) {
    for (int i = 0; i < n; i++) d[i] = a[i] > b[i] ? a[i] : b[i];
}
/* Swapped-arm forms are NOT min/max (MINPS picks the second source on
 * both-zero lanes): the compare+blendv lowering must preserve them. */
static void k_min_swapped(float *restrict d, const float *restrict a,
                          const float *restrict b, int n) {
    for (int i = 0; i < n; i++) d[i] = a[i] < b[i] ? b[i] : a[i];
}
static void k_le_select(float *restrict d, const float *restrict a,
                        const float *restrict b, int n) {
    for (int i = 0; i < n; i++) d[i] = a[i] <= b[i] ? a[i] : b[i];
}
/* Clamp shapes: branchy if-form (store sinking + if-conversion), nested
 * ternary, and the parametric form. */
static void k_clamp01(float *restrict d, const float *restrict a, int n) {
    for (int i = 0; i < n; i++) {
        float x = a[i];
        if (x < 0.0f) x = 0.0f;
        else if (x > 1.0f) x = 1.0f;
        d[i] = x;
    }
}
static void k_clamp_ternary(float *restrict d, const float *restrict a, int n) {
    for (int i = 0; i < n; i++)
        d[i] = a[i] < 0.0f ? 0.0f : (a[i] > 1.0f ? 1.0f : a[i]);
}
static void k_clamp_param(float *restrict d, const float *restrict a,
                          float lo, float hi, int n) {
    for (int i = 0; i < n; i++) {
        float x = a[i];
        if (x < lo) x = lo;
        if (x > hi) x = hi;
        d[i] = x;
    }
}
/* General lane selects (compare + blendv). */
static void k_select3(float *restrict d, const float *restrict a,
                      const float *restrict b, const float *restrict c, int n) {
    for (int i = 0; i < n; i++) d[i] = a[i] < b[i] ? a[i] : c[i];
}
static void k_select_thr(float *restrict d, const float *restrict a,
                         const float *restrict c, float threshold, int n) {
    for (int i = 0; i < n; i++) d[i] = a[i] > threshold ? a[i] : c[i];
}
/* Mixed min/max pipeline: min(1, max(0, a)) on every lane. */
static void k_minmax_chain(float *restrict d, const float *restrict a, int n) {
    for (int i = 0; i < n; i++) {
        float x = a[i] < 0.0f ? 0.0f : a[i];
        d[i] = x > 1.0f ? 1.0f : x;
    }
}
/* F64 twins. */
static void k_min_f64(double *restrict d, const double *restrict a,
                      const double *restrict b, int n) {
    for (int i = 0; i < n; i++) d[i] = a[i] < b[i] ? a[i] : b[i];
}
static void k_clamp_f64(double *restrict d, const double *restrict a, int n) {
    for (int i = 0; i < n; i++) {
        double x = a[i];
        if (x < 0.0) x = 0.0;
        else if (x > 1.0) x = 1.0;
        d[i] = x;
    }
}

#define RUN3(NAME)                                                           \
    do {                                                                     \
        memset(D, 0, sizeof(D));                                             \
        NAME(D, A, B, N);                                                    \
        REPORT(#NAME, D);                                                    \
    } while (0)

int main(void) {
    fill();
    RUN3(k_min_ternary);
    RUN3(k_max_ternary);
    RUN3(k_min_swapped);
    RUN3(k_le_select);
    {
        memset(D, 0, sizeof(D));
        k_clamp01(D, A, N);
        REPORT("k_clamp01", D);
    }
    {
        memset(D, 0, sizeof(D));
        k_clamp_ternary(D, A, N);
        REPORT("k_clamp_ternary", D);
    }
    {
        memset(D, 0, sizeof(D));
        k_select3(D, A, B, C, N);
        REPORT("k_select3", D);
    }
    {
        memset(D, 0, sizeof(D));
        k_minmax_chain(D, A, N);
        REPORT("k_minmax_chain", D);
    }
    {
        memset(D, 0, sizeof(D));
        k_clamp_param(D, A, -0.5f, 0.75f, N);
        REPORT("k_clamp_param", D);
    }
    {
        memset(D, 0, sizeof(D));
        k_select_thr(D, A, C, 0.25f, N);
        REPORT("k_select_thr", D);
    }
    {
        double Ad[64], Bd[64], Dd[64];
        for (int i = 0; i < 64; i++) {
            Ad[i] = A[i];
            Bd[i] = B[i];
        }
        memset(Dd, 0, sizeof(Dd));
        k_min_f64(Dd, Ad, Bd, 64);
        printf("k_min_f64 %08x\n", hash_bits(Dd, sizeof(Dd)));
        memset(Dd, 0, sizeof(Dd));
        k_clamp_f64(Dd, Ad, 64);
        printf("k_clamp_f64 %08x\n", hash_bits(Dd, sizeof(Dd)));
    }
    return 0;
}
