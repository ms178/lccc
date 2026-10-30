/* Strict-FP reduction unrolling: the arithmetic must not move.
 *
 * `num_serial_f32` / `num_serial_f64` are the corpus's `float sum(const float
 * *, int)` shape: one carried accumulator, a constant trip once the callee's
 * `n` parameter is specialized.  Unrolling them k times is legal for strict FP
 * ONLY because the k copies stay a single accumulation chain (the operation
 * order is byte-identical); every check below is chosen so a reassociated or
 * vectorized rewrite changes the answer.
 *
 *   * `assoc_seq` sums values whose exact order matters: the repeating
 *     1.0, 1e8, -1e8 triple evaluates to 0.0 in source order (the leading
 *     1.0 is swallowed when it meets 1e8) and the array ends on its own
 *     1.0, so the answer is 1.0.  Any tree that pairs 1e8 with -1e8 first
 *     keeps every 1.0 and lands on 22.0 instead.
 *   * `num_tail_*` use trips that are NOT multiples of the unroll factor, so a
 *     transform that assumed divisibility would drop or double the tail.
 *   * `num_dynamic` is called with a runtime bound (stays rolled) and with
 *     a literal (may specialize to a constant trip); either way the answer
 *     must stay exact.
 *   * `num_dot_serial` is the multiply-accumulate chain (dot product), the
 *     second shape the reduction unroller admits.
 *
 * The reference values are computed here by plain sequential loops (the
 * association payload reads a `volatile` array), so both computations share
 * one source and one compiler; only the loop shape differs.
 */
#include <stdio.h>

#if defined(__GNUC__)
#define NOINLINE __attribute__((noinline))
#else
#define NOINLINE
#endif

#define N 1024

static float fa[N], fb[N];
static double da[N], db[N];
static volatile float fseq_src[N];
static volatile double dseq_src[N];

NOINLINE static float num_serial_f32(const float *restrict v, int n) {
    float s = 0.0f;
    for (int i = 0; i < n; ++i)
        s += v[i];
    return s;
}

NOINLINE static double num_serial_f64(const double *restrict v, int n) {
    double s = 0.0;
    for (int i = 0; i < n; ++i)
        s += v[i];
    return s;
}

NOINLINE static float num_dot_serial(const float *restrict a, const float *restrict b,
                                    int n) {
    float s = 0.0f;
    for (int i = 0; i < n; ++i)
        s += a[i] * b[i];
    return s;
}

/* Shape-contract closures.  `sum_lit_*` / `dot_lit` carry a literal trip;
 * `sum_once_f32` takes `n` as a parameter and has exactly ONE call site with a
 * literal, so its trip only becomes constant once interprocedural constant
 * propagation specializes the callee — the pass-ordering half of the fix. */
NOINLINE static float sum_lit_f32(const float *restrict v) {
    float s = 0.0f;
    for (int i = 0; i < N; ++i)
        s += v[i];
    return s;
}

NOINLINE static double sum_lit_f64(const double *restrict v) {
    double s = 0.0;
    for (int i = 0; i < N; ++i)
        s += v[i];
    return s;
}

NOINLINE static float dot_lit(const float *restrict a, const float *restrict b) {
    float s = 0.0f;
    for (int i = 0; i < N; ++i)
        s += a[i] * b[i];
    return s;
}

NOINLINE static float sum_once_f32(const float *restrict v, int n) {
    float s = 0.0f;
    for (int i = 0; i < n; ++i)
        s += v[i];
    return s;
}

/* Runtime bound: same body, no constant trip. */
NOINLINE static float num_dynamic(const float *restrict v, int n) { return num_serial_f32(v, n); }

static int fail;

static void check_u32(const char *what, unsigned got, unsigned want) {
    if (got != want) {
        printf("FAIL %s: got %u want %u\n", what, got, want);
        fail = 1;
    }
}

static void check_u64(const char *what, unsigned long long got,
                      unsigned long long want) {
    if (got != want) {
        printf("FAIL %s: got %016llx want %016llx\n", what, got, want);
        fail = 1;
    }
}

static int bits32(float f) {
    union { float f; unsigned u; } v;
    v.f = f;
    return (int)v.u;
}

static unsigned long long bits64(double d) {
    union { double d; unsigned long long u; } v;
    v.d = d;
    return v.u;
}

int main(void) {
    for (int i = 0; i < N; ++i) {
        fa[i] = (float)(i & 31) * 0.25f;
        fb[i] = (float)(i & 7) * 0.125f;
        da[i] = (double)(i & 31) * 0.25;
        db[i] = (double)(i & 7) * 0.125;
        fseq_src[i] = fa[i];
        dseq_src[i] = da[i];
    }

    /* 1. Association-sensitive payload: 1.0 + 1e8 - 1e8 ... keeps the order. */
    for (int i = 0; i < 64; ++i)
        fseq_src[i] = (i % 3 == 0) ? 1.0f : (i % 3 == 1 ? 1e8f : -1e8f);
    for (int i = 0; i < 64; ++i)
        dseq_src[i] = (i % 3 == 0) ? 1.0 : (i % 3 == 1 ? 1e8 : -1e8);

    /* Sequential references (volatile source, one element at a time). */
    {
        float s = 0.0f;
        for (int i = 0; i < 64; ++i)
            s += fseq_src[i];
        printf("%08x\n", (unsigned)bits32(s));
        check_u32("serial_f32_assoc", (unsigned)bits32(num_serial_f32((const float *)fseq_src, 64)),
                  (unsigned)bits32(s));
    }
    {
        double s = 0.0;
        for (int i = 0; i < 64; ++i)
            s += dseq_src[i];
        printf("%016llx\n", bits64(s));
        check_u64("serial_f64_assoc", bits64(num_serial_f64((const double *)dseq_src, 64)),
                  bits64(s));
    }

    /* 2. Plain in-order sums over the full array, divisible trip. */
    {
        float s = 0.0f;
        for (int i = 0; i < N; ++i)
            s += fa[i];
        check_u32("serial_f32", (unsigned)bits32(num_serial_f32(fa, N)), (unsigned)bits32(s));
        printf("%08x\n", (unsigned)bits32(s));
    }
    {
        double s = 0.0;
        for (int i = 0; i < N; ++i)
            s += da[i];
        check_u64("serial_f64", bits64(num_serial_f64(da, N)), bits64(s));
        printf("%016llx\n", bits64(s));
    }
    {
        float s = 0.0f;
        for (int i = 0; i < N; ++i)
            s += fa[i] * fb[i];
        check_u32("dot_serial", (unsigned)bits32(num_dot_serial(fa, fb, N)),
                  (unsigned)bits32(s));
        printf("%08x\n", (unsigned)bits32(s));
    }

    /* 2b. The shape-contract closures must agree with the sequential sums. */
    {
        float s = 0.0f;
        for (int i = 0; i < N; ++i)
            s += fa[i];
        check_u32("sum_lit_f32", (unsigned)bits32(sum_lit_f32(fa)), (unsigned)bits32(s));
        /* The single-call-site callee: ONE literal call site, so IPCP can
         * specialize the trip (this is the pass-ordering pin). */
        check_u32("sum_once_f32", (unsigned)bits32(sum_once_f32(fa, N)),
                  (unsigned)bits32(s));
        float d = 0.0f;
        for (int i = 0; i < N; ++i)
            d += fa[i] * fb[i];
        check_u32("dot_lit", (unsigned)bits32(dot_lit(fa, fb)), (unsigned)bits32(d));
        double g = 0.0;
        for (int i = 0; i < N; ++i)
            g += da[i];
        check_u64("sum_lit_f64", bits64(sum_lit_f64(da)), bits64(g));
    }

    /* 3. Tails: every trip residue modulo 8 around the full array. */
    for (int n = N - 8; n <= N; ++n) {
        float s = 0.0f;
        for (int i = 0; i < n; ++i)
            s += fa[i];
        check_u32("serial_f32_tail", (unsigned)bits32(num_serial_f32(fa, n)),
                  (unsigned)bits32(s));
        double d = 0.0;
        for (int i = 0; i < n; ++i)
            d += da[i];
        check_u64("serial_f64_tail", bits64(num_serial_f64(da, n)), bits64(d));
    }

    /* 4. Runtime bound, including the empty and single-element trips. */
    for (int n = 0; n <= 3; ++n)
        check_u32("dynamic", (unsigned)bits32(num_dynamic(fa, n)),
                  (unsigned)bits32(num_serial_f32(fa, n)));
    check_u32("dynamic_full", (unsigned)bits32(num_dynamic(fa, N)),
              (unsigned)bits32(num_serial_f32(fa, N)));

    if (fail)
        return 1;
    printf("ok\n");
    return 0;
}
