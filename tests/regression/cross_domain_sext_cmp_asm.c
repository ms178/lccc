/*
 * Emission shapes for the cross-domain comparison rewrite
 * (simplify's `cross_domain_sign_test`).  No headers: the gate compiles this
 * for every backend it has a driver for, and the cross targets have no host
 * libc.  The values are checked by cross_domain_sext_cmp.c.
 *
 * `fire*` must become a sign test; `keep*` must stay a compare against the
 * constant.  Every `keep` is one step outside the proven band, so a rule that
 * widened its guard by one would fail here.
 */

/* 8-bit source, constant between the halves: `(unsigned)(signed char)x < 300`
   is `x >=s 0`. */
int fire_lt(int x) { return (unsigned)(signed char)x < 300; }
int fire_le(int x) { return (unsigned)(signed char)x <= 300; }
int fire_ge(int x) { return (unsigned)(signed char)x >= 300; }
int fire_gt(int x) { return (unsigned)(signed char)x > 300; }

/* 16-bit source, constant between the halves. */
int fire16_ge(int x) { return (unsigned)(short)x >= 70000; }

/* 64-bit compare width: `U - 2^31` is the negative half's first member. */
int fire64_ge(long long x) { return (unsigned long long)(int)x >= 18446744071562067968ULL; }
int fire64_gt(long long x) { return (unsigned long long)(int)x > 18446744071562067967ULL; }

/* Constants inside the non-negative half: the answer depends on the value, so
   the compare must survive. */
int keep_in_half(int x) { return (unsigned)(signed char)x < 5; }
int keep16_in_half(int x) { return (unsigned)(short)x < 300; }

/* One past the negative half's first member: the negative half is no longer
   entirely above the constant, so half of it answers each way. */
int keep_gap(int x) { return (unsigned)(signed char)x >= 4294967169u; }
int keep64_gap(long long x) { return (unsigned long long)(int)x >= 18446744071562067969ULL; }

/* Above the last member: everything is below it. */
int keep64_high(long long x) { return (unsigned long long)(int)x <= 18446744073709551615ULL; }

/* The suite builds and runs every file in this directory against the GCC
   differential, so this one drives its own shapes rather than being
   emission-only: the values must agree with GCC's, which the differential
   checks.  The exhaustive model lives in cross_domain_sext_cmp.c. */
static int failures;
static void expect(int got, int want, const char *what) {
    if (got != want) {
        ++failures;
        (void)what;
    }
}
int main(void) {
    volatile int xs[] = {0, 1, -1, 127, -128, 128, 300, -300, 255, -255};
    volatile long long ws[] = {0, 1, -1, 2147483647LL, -2147483648LL, 4294967295LL};
    for (unsigned i = 0; i < sizeof xs / sizeof xs[0]; i++) {
        int x = xs[i];
        /* `(unsigned)(signed char)x < 300` is `x`'s low byte being
           non-negative, spelled without the naive rewrite. */
        expect(fire_lt(x), (signed char)x >= 0, "fire_lt");
        expect(fire_le(x), (signed char)x >= 0, "fire_le");
        expect(fire_ge(x), (signed char)x < 0, "fire_ge");
        expect(fire_gt(x), (signed char)x < 0, "fire_gt");
        expect(fire16_ge(x), (short)x < 0, "fire16_ge");
        expect(keep_in_half(x), (unsigned)(signed char)x < 5u, "keep_in_half");
        expect(keep16_in_half(x), (unsigned)(short)x < 300u, "keep16_in_half");
        expect(keep_gap(x), (unsigned)(signed char)x >= 4294967169u, "keep_gap");
    }
    for (unsigned i = 0; i < sizeof ws / sizeof ws[0]; i++) {
        long long w = ws[i];
        expect(fire64_ge(w), (int)w < 0, "fire64_ge");
        expect(fire64_gt(w), (int)w < 0, "fire64_gt");
        expect(keep64_gap(w), (unsigned long long)(int)w >= 18446744071562067969ULL,
               "keep64_gap");
        expect(keep64_high(w), (unsigned long long)(int)w <= 18446744073709551615ULL,
               "keep64_high");
    }
    return failures;
}
