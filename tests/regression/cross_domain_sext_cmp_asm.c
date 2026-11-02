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
