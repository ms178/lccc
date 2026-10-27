/*
 * Byte-predicate counting reduction (vpsadbw path) — exhaustive gate.
 *
 * Covers the transform's correctness envelope and, deliberately, the
 * shapes it must REFUSE (fail closed):
 *   - every trip count 0..N with N prime, so the packed body, the scalar
 *     remainder and the empty loop are all covered (the remainder drop
 *     bug this file pins: N=3000 dropped the last 24 elements);
 *   - non-zero IV start, IV step 2, strided (`s[2*i]`) and shifted
 *     (`s[i+7]`) addressing: each must stay scalar — a packed body would
 *     read the wrong elements;
 *   - a zero-trip guard bypass: the accumulator keeps its entry value;
 *   - a non-zero accumulator init feeding the result;
 *   - signed (I8) streams and signed predicates;
 *   - a compare-with-range predicate (2-window conjunction);
 *   - `!=` counts (packed as one `==` compare, complemented at the exit)
 *     and `== 0` counts (the raw mask summed, scaled out at the exit);
 *   - if-converted counts over GLOBAL arrays whose address is materialised
 *     inside the loop (`if (g[i]) c++`, the sieve's shape), at start 0 and
 *     at a nonzero start, over several array contents.
 *
 * The reference is computed through `volatile` scalars so it can never be
 * vectorized into the same (possibly wrong) form.
 */
#include <stdio.h>
#include <string.h>
#include <stdlib.h>

#define N 1031  /* prime: odd remainder for both VF=16 and VF=32 */
#define N2 3000 /* the exact remainder-drop shape: 3000 = 93*32 + 24 */

static unsigned char usrc[N];
static signed char   ssrc[N];

/* ---- kernels the transform must handle (vectorize or refuse, never
 *      miscompile) ------------------------------------------------------ */
unsigned long k_count_eq(const unsigned char *restrict s, unsigned long n) {
    unsigned long c = 0;
    for (unsigned long i = 0; i < n; ++i) c += (s[i] == 0x5A);
    return c;
}
unsigned long k_count_range(const unsigned char *restrict s, unsigned long n) {
    unsigned long c = 0;
    for (unsigned long i = 0; i < n; ++i) c += (s[i] >= 'A' && s[i] <= 'Z');
    return c;
}
unsigned long k_count_signed(signed char *restrict s, unsigned long n) {
    unsigned long c = 0;
    for (unsigned long i = 0; i < n; ++i) c += (s[i] < -3);
    return c;
}
unsigned long k_count_init(const unsigned char *restrict s, unsigned long n, unsigned long seed) {
    unsigned long c = seed; /* non-zero accumulator entry value */
    for (unsigned long i = 0; i < n; ++i) c += (s[i] != 0);
    return c;
}
unsigned long k_count_guard(const unsigned char *restrict s, unsigned long n) {
    unsigned long c = 0;
    if (n > 4) { for (unsigned long i = 0; i < n; ++i) c += (s[i] > 127); }
    return c; /* zero-trip bypass path must keep c = 0 */
}
/* 3000-element shape: the historical remainder-drop reproducer. */
unsigned long k_count_3000(const unsigned char *restrict s) {
    unsigned long c = 0;
    for (unsigned long i = 0; i < N2; ++i) c += (s[i] == 0x7F);
    return c;
}
/* Commutative accumulator spelling (`c = v + c`): the counting matcher
 * accepts the loop-carried phi on EITHER side of the add — same math as
 * k_count_eq, written the other way round. */
unsigned long k_count_rhs(const unsigned char *restrict s, unsigned long n) {
    unsigned long c = 0;
    for (unsigned long i = 0; i < n; ++i) c = (s[i] == 0x5A) + c;
    return c;
}
/* Signed dynamic trip limit: vector-bound formation must use truncation
 * toward zero, so negative limits produce no packed or scalar iterations. */
unsigned long k_count_signed_limit(const unsigned char *restrict s, int n) {
    unsigned long c = 0;
    for (int i = 0; i < n; ++i) c += (s[i] == 0x5A);
    return c;
}

/* ---- kernels the transform must REFUSE (wrong elements if packed) ---- */
unsigned long k_refuse_stride(const unsigned char *restrict s, unsigned long n) {
    unsigned long c = 0;
    for (unsigned long i = 0; i < n / 2; ++i) c += (s[2 * i] == 0x5A);
    return c;
}
unsigned long k_refuse_shift(const unsigned char *restrict s, unsigned long n) {
    unsigned long c = 0;
    for (unsigned long i = 0; i + 7 < n; ++i) c += (s[i + 7] == 0x5A);
    return c;
}
unsigned long k_refuse_start7(const unsigned char *restrict s, unsigned long n) {
    unsigned long c = 0;
    for (unsigned long i = 7; i < n; ++i) c += (s[i] == 0x5A);
    return c;
}
unsigned long k_refuse_step2(const unsigned char *restrict s, unsigned long n) {
    unsigned long c = 0;
    for (unsigned long i = 0; i < n; i += 2) c += (s[i] == 0x5A);
    return c;
}

/* Constant-bound counts over globals: `g[i]` lowers to an in-loop symbol
 * address that the count normaliser rematerialises in the preheader. */
static unsigned char gsrc[N];
static unsigned char gsrc2[N];
__attribute__((noinline)) int k_glob_nz(void) {
    int c = 0;
    for (int i = 2; i <= N - 1; i++) if (gsrc[i]) c++;
    return c;
}
__attribute__((noinline)) int k_glob_z(void) {
    int c = 0;
    for (int i = 2; i <= N - 1; i++) if (gsrc[i] == 0) c++;
    return c;
}
__attribute__((noinline)) int k_glob_eq0(void) {
    int c = 0;
    for (int i = 0; i < N; i++) if (gsrc[i] == 0x5A) c++;
    return c;
}
__attribute__((noinline)) int k_glob_ne2(void) {
    int c = 0;
    for (int i = 1; i < N; i++) if (gsrc[i] != gsrc2[i]) c++;
    return c;
}
unsigned long k_count_ne(const unsigned char *restrict s, unsigned long n) {
    unsigned long c = 0;
    for (unsigned long i = 0; i < n; ++i) c += (s[i] != 0x5A);
    return c;
}
unsigned long k_count_zero(const unsigned char *restrict s, unsigned long n) {
    unsigned long c = 0;
    for (unsigned long i = 0; i < n; ++i) c += (s[i] == 0);
    return c;
}

/* ---- volatile-scalar references (never vectorizable) ----------------- */
static int v_glob(int lo, int hi, int mode) {
    int c = 0;
    for (int i = lo; i <= hi; i++) {
        volatile unsigned char v = gsrc[i], w = gsrc2[i];
        c += mode == 0 ? v != 0 : mode == 1 ? v == 0 : mode == 2 ? v == 0x5A : v != w;
    }
    return c;
}
static unsigned long v_count_ne(const unsigned char *s, unsigned long n) {
    unsigned long c = 0;
    for (unsigned long i = 0; i < n; ++i) { volatile unsigned char v = s[i]; c += (v != 0x5A); }
    return c;
}
static unsigned long v_count_zero(const unsigned char *s, unsigned long n) {
    unsigned long c = 0;
    for (unsigned long i = 0; i < n; ++i) { volatile unsigned char v = s[i]; c += (v == 0); }
    return c;
}
static unsigned long v_count_eq(const unsigned char *s, unsigned long n) {
    unsigned long c = 0;
    for (unsigned long i = 0; i < n; ++i) { volatile unsigned char v = s[i]; c += (v == 0x5A); }
    return c;
}
static unsigned long v_count_range(const unsigned char *s, unsigned long n) {
    unsigned long c = 0;
    for (unsigned long i = 0; i < n; ++i) { volatile unsigned char v = s[i]; c += (v >= 'A' && v <= 'Z'); }
    return c;
}
static unsigned long v_count_signed(signed char *s, unsigned long n) {
    unsigned long c = 0;
    for (unsigned long i = 0; i < n; ++i) { volatile signed char v = s[i]; c += (v < -3); }
    return c;
}
static unsigned long v_count_init(const unsigned char *s, unsigned long n, unsigned long seed) {
    unsigned long c = seed;
    for (unsigned long i = 0; i < n; ++i) { volatile unsigned char v = s[i]; c += (v != 0); }
    return c;
}
static unsigned long v_count_guard(const unsigned char *s, unsigned long n) {
    unsigned long c = 0;
    if (n > 4) { for (unsigned long i = 0; i < n; ++i) { volatile unsigned char v = s[i]; c += (v > 127); } }
    return c;
}
static unsigned long v_count_3000(const unsigned char *s) {
    unsigned long c = 0;
    for (unsigned long i = 0; i < N2; ++i) { volatile unsigned char v = s[i]; c += (v == 0x7F); }
    return c;
}
static unsigned long v_count_rhs(const unsigned char *s, unsigned long n) {
    unsigned long c = 0;
    for (unsigned long i = 0; i < n; ++i) { volatile unsigned char v = s[i]; c = (v == 0x5A) + c; }
    return c;
}
static unsigned long v_count_signed_limit(const unsigned char *s, int n) {
    unsigned long c = 0;
    for (int i = 0; i < n; ++i) { volatile unsigned char v = s[i]; c += (v == 0x5A); }
    return c;
}
static unsigned long v_refuse_stride(const unsigned char *s, unsigned long n) {
    unsigned long c = 0;
    for (unsigned long i = 0; i < n / 2; ++i) { volatile unsigned char v = s[2 * i]; c += (v == 0x5A); }
    return c;
}
static unsigned long v_refuse_shift(const unsigned char *s, unsigned long n) {
    unsigned long c = 0;
    for (unsigned long i = 0; i + 7 < n; ++i) { volatile unsigned char v = s[i + 7]; c += (v == 0x5A); }
    return c;
}
static unsigned long v_refuse_start7(const unsigned char *s, unsigned long n) {
    unsigned long c = 0;
    for (unsigned long i = 7; i < n; ++i) { volatile unsigned char v = s[i]; c += (v == 0x5A); }
    return c;
}
static unsigned long v_refuse_step2(const unsigned char *s, unsigned long n) {
    unsigned long c = 0;
    for (unsigned long i = 0; i < n; i += 2) { volatile unsigned char v = s[i]; c += (v == 0x5A); }
    return c;
}

int main(void) {
    /* Byte-exhaustive prefix + pseudo-random tail. */
    for (int i = 0; i < 256 && i < N; i++) usrc[i] = (unsigned char)i;
    for (int i = 256; i < N; i++) usrc[i] = (unsigned char)(i * 131 + 17);
    for (int i = 0; i < N; i++) ssrc[i] = (signed char)(usrc[i] ^ 0x9C);

    int fails = 0;
#define CHECK(expr, ref, name) do { \
        unsigned long _e = (expr), _r = (ref); \
        if (_e != _r) { printf("FAIL %s: %lu != %lu\n", name, _e, _r); fails++; } \
    } while (0)

    /* Every trip count from 0..N: packed body + remainder + empty loop. */
    for (unsigned long n = 0; n <= N; n++) {
        CHECK(k_count_eq(usrc, n), v_count_eq(usrc, n), "count_eq");
        CHECK(k_count_rhs(usrc, n), v_count_rhs(usrc, n), "count_rhs");
        CHECK(k_count_range(usrc, n), v_count_range(usrc, n), "count_range");
        CHECK(k_count_signed(ssrc, n), v_count_signed(ssrc, n), "count_signed");
        CHECK(k_count_init(usrc, n, 12345), v_count_init(usrc, n, 12345), "count_init");
        CHECK(k_count_ne(usrc, n), v_count_ne(usrc, n), "count_ne");
        CHECK(k_count_zero(usrc, n), v_count_zero(usrc, n), "count_zero");
        CHECK(k_count_guard(usrc, n), v_count_guard(usrc, n), "count_guard");
        CHECK(k_refuse_stride(usrc, n), v_refuse_stride(usrc, n), "refuse_stride");
        CHECK(k_refuse_shift(usrc, n), v_refuse_shift(usrc, n), "refuse_shift");
        CHECK(k_refuse_start7(usrc, n), v_refuse_start7(usrc, n), "refuse_start7");
        CHECK(k_refuse_step2(usrc, n), v_refuse_step2(usrc, n), "refuse_step2");
    }
    /* A signed dynamic limit can be negative; every such case must be an
     * empty loop even after the vector bound is hoisted out of the loop. */
    {
        static const int signed_limits[] = { -1031, -65, -33, -32, -31, -1, 0, 1, 31, 32, 33, N };
        for (size_t i = 0; i < sizeof(signed_limits) / sizeof(signed_limits[0]); ++i) {
            const int n = signed_limits[i];
            CHECK(k_count_signed_limit(usrc, n), v_count_signed_limit(usrc, n), "count_signed_limit");
        }
    }
    /* Global-array counts over contents that hit every lane position:
     * all clear, all set, every byte value, sparse needles, and a stride
     * coprime to both widths. */
    for (int fill = 0; fill < 5; fill++) {
        for (int i = 0; i < N; i++) {
            unsigned char b = fill == 0   ? 0
                              : fill == 1 ? 0xFF
                              : fill == 2 ? usrc[i]
                              : fill == 3 ? (i % 97 == 0 ? 0x5A : 0)
                                          : (i % 5 == 0 ? 0 : (unsigned char)(i | 1));
            gsrc[i] = b;
            gsrc2[i] = (unsigned char)(fill & 1 ? b : b ^ (i % 3 == 0));
        }
        CHECK(k_glob_nz(), v_glob(2, N - 1, 0), "glob_nz");
        CHECK(k_glob_z(), v_glob(2, N - 1, 1), "glob_z");
        CHECK(k_glob_eq0(), v_glob(0, N - 1, 2), "glob_eq0");
        CHECK(k_glob_ne2(), v_glob(1, N - 1, 3), "glob_ne2");
    }
    /* The exact historical remainder-drop shape. */
    {
        static unsigned char big[N2];
        for (int i = 0; i < N2; i++) big[i] = (i % 37 == 0) ? 0x7F : (unsigned char)(i * 7 + 1);
        CHECK(k_count_3000(big), v_count_3000(big), "count_3000");
    }
    if (fails == 0) printf("ALL OK\n");
    return fails != 0;
}
