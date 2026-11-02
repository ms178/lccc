/*
 * Two miscompiles in the same family: a vectorizer that drops a second
 * loop-carried value, and a data emitter that drops a sign bit.
 *
 * 1. NON-IV LOOP-CARRIED VALUE. The packed map/stencil body runs ONE
 *    iteration per W elements and reconstructs no scalar recurrence, so any
 *    loop-carried value other than the counter ends the loop holding its
 *    value after n/W steps instead of n. Measured on the unfixed compiler at
 *    -O2/-O3 with an 8-lane body:
 *
 *      for (i) { d[i] = a[i]*2; acc += 3; } return acc;   ->  3*(n/8), 0 at n=1
 *      for (i) { d[i] = *p++ * 2; }        return p - a;  ->  n/8
 *
 *    Both analyzers needed the guard: rejecting the shape in the map
 *    analyzer alone just handed it to the stencil analyzer, which
 *    miscompiled it identically.
 *
 * 2. NEGATIVE ZERO IN STATIC DATA. The array emitter collapsed runs of
 *    "zero" elements into `.zero N` using C truthiness (`is_zero`), under
 *    which -0.0 IS zero. `static const float t[] = {-0.0f}` therefore read
 *    back as +0.0f, so `1/t[0]` gave +inf, `signbit` gave 0, and any
 *    MINPS/MAXPS lane selection on the value flipped.
 *
 * The recurrence kernels sweep n across every vector/remainder split so a
 * partially-executed packed body cannot hide, and the -0.0 checks look at
 * raw bits rather than comparing against 0.0 (which would be true either
 * way).
 */
#include <stdio.h>
#include <string.h>

static int fails;
#define CHECK(cond, ...)                                                                 \
    do {                                                                                 \
        if (!(cond)) {                                                                   \
            printf(__VA_ARGS__);                                                         \
            ++fails;                                                                     \
        }                                                                                \
    } while (0)

/* ---------- 1. loop-carried values other than the IV ---------- */

__attribute__((noinline)) static long k_acc(float *restrict d, const float *restrict a, long n) {
    long acc = 0;
    for (long i = 0; i < n; i++) {
        d[i] = a[i] * 2.0f;
        acc += 3;
    }
    return acc;
}

__attribute__((noinline)) static long k_ptr(float *restrict d, const float *restrict a, long n) {
    const float *p = a;
    for (long i = 0; i < n; i++) {
        d[i] = *p * 2.0f;
        p++;
    }
    return (long) (p - a);
}

/* Two extra recurrences at once, one of them multiplicative. */
__attribute__((noinline)) static long k_two(float *restrict d, const float *restrict a, long n) {
    long s = 0, m = 1;
    for (long i = 0; i < n; i++) {
        d[i] = a[i] + 1.0f;
        s += i;
        m = m * 2 - 1;
    }
    return s * 100 + m;
}

/* A conditional recurrence: the count must be per ELEMENT, not per lane. */
__attribute__((noinline)) static long k_count(float *restrict d, const float *restrict a, long n) {
    long c = 0;
    for (long i = 0; i < n; i++) {
        float x = a[i];
        d[i] = x < 0.0f ? 0.0f : x;
        c += 1;
    }
    return c;
}

/* The shape that SHOULD still vectorize: no second carried value. */
__attribute__((noinline)) static void k_pure(float *restrict d, const float *restrict a, long n) {
    for (long i = 0; i < n; i++)
        d[i] = a[i] * 2.0f;
}

/* ---------- 2. negative zero in static initializers ---------- */

static const float fz[8] = {-0.0f, 0.0f, -0.0f, -0.0f, 0.0f, 1.0f, -0.0f, 0.0f};
static const double dz[4] = {-0.0, 0.0, -0.0, 2.0};
static const float all_neg[4] = {-0.0f, -0.0f, -0.0f, -0.0f};
static const long double ldz[2] = {-0.0L, 0.0L};
/* A zero run that really is all-zero must still collapse correctly. */
static const float pz[6] = {0.0f, 0.0f, 0.0f, 0.0f, 0.0f, 3.0f};

static unsigned f2b(float f) {
    unsigned u;
    memcpy(&u, &f, sizeof u);
    return u;
}
static unsigned long long d2b(double d) {
    unsigned long long u;
    memcpy(&u, &d, sizeof u);
    return u;
}

int main(void) {
    enum { N = 80 };
    static float A[N], D[N];
    for (int i = 0; i < N; i++)
        A[i] = (float) (i - 40) * 0.5f;

    for (long n = 0; n <= 40; n++) {
        memset(D, 0, sizeof D);
        CHECK(k_acc(D, A, n) == 3 * n, "k_acc(%ld) = %ld want %ld\n", n, k_acc(D, A, n), 3 * n);
        CHECK(k_ptr(D, A, n) == n, "k_ptr(%ld) = %ld want %ld\n", n, k_ptr(D, A, n), n);
        CHECK(k_count(D, A, n) == n, "k_count(%ld) = %ld want %ld\n", n, k_count(D, A, n), n);

        long s = 0, m = 1;
        for (long i = 0; i < n; i++) {
            s += i;
            m = m * 2 - 1;
        }
        CHECK(k_two(D, A, n) == s * 100 + m, "k_two(%ld) wrong\n", n);

        /* The pure map must still be value-correct. */
        memset(D, 0, sizeof D);
        k_pure(D, A, n);
        for (long i = 0; i < n; i++)
            CHECK(D[i] == A[i] * 2.0f, "k_pure(%ld)[%ld] wrong\n", n, i);
    }

    /* -0.0 must survive as 0x80000000 / 0x8000000000000000. */
    for (int i = 0; i < 8; i++) {
        unsigned want = (i == 0 || i == 2 || i == 3 || i == 6)  ? 0x80000000u
                        : (i == 5)                              ? 0x3f800000u
                                                                : 0u;
        CHECK(f2b(fz[i]) == want, "fz[%d] bits=%08x want %08x\n", i, f2b(fz[i]), want);
    }
    CHECK(d2b(dz[0]) == 0x8000000000000000ULL, "dz[0] lost its sign\n");
    CHECK(d2b(dz[2]) == 0x8000000000000000ULL, "dz[2] lost its sign\n");
    CHECK(d2b(dz[1]) == 0ULL, "dz[1] should be +0\n");
    for (int i = 0; i < 4; i++)
        CHECK(f2b(all_neg[i]) == 0x80000000u, "all_neg[%d] lost its sign\n", i);
    CHECK(__builtin_signbit(ldz[0]) != 0, "ldz[0] (long double -0.0) lost its sign\n");
    CHECK(__builtin_signbit(ldz[1]) == 0, "ldz[1] should be +0\n");
    for (int i = 0; i < 5; i++)
        CHECK(f2b(pz[i]) == 0u, "pz[%d] should be +0\n", i);
    CHECK(f2b(pz[5]) == 0x40400000u, "pz[5] should be 3.0f\n");

    /* Sign-sensitive consumers must agree. */
    CHECK(1.0f / fz[0] < 0.0f, "1/-0.0f should be -inf\n");
    CHECK(1.0 / dz[0] < 0.0, "1/-0.0 should be -inf\n");
    CHECK(__builtin_copysignf(1.0f, fz[0]) == -1.0f, "copysign(-0.0f) wrong\n");

    printf("vectorize_carried_value_and_negzero: %s\n", fails ? "FAIL" : "OK");
    return fails != 0;
}
