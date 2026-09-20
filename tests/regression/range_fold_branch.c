/*
 * Branch-condition range fusion: runtime semantics battery.
 *
 * The range_fold pass folds the SHORT-CIRCUIT BRANCH form of
 * `x >= lo && x <= hi` / `x < lo || x > hi` into the unsigned-bias test
 * `(unsigned)(x - lo) <= hi - lo` when the guarded body keeps the chain a
 * two-block CFG shape (big bodies, loop guards — everything if-conversion
 * declines). This battery proves the fold's math per input value: every
 * foldable shape is paired with a hand-written oracle in the already-folded
 * algebraic form, computed by the plain sub/cmp machinery rather than the
 * folding pass, and any disagreement fails the run.
 *
 * Adversarial shapes pin the rejection discipline: different values, mixed
 * signedness, unrepresentable spans, value-context uses of the compares,
 * side effects inside the second condition, and two-predecessor check
 * blocks must all keep exact C semantics (folded or not, the results are
 * the contract; the assembly-level no-fold proofs live in
 * check_range_fold_branch.sh).
 *
 * Exit code 0 = every shape agreed with its oracle on every swept value.
 * The printed hash additionally allows the suite's GCC differential to
 * catch an oracle-side mistake.
 *
 * Oracle discipline: every hand oracle computes its bias subtraction in
 * the UNSIGNED domain of the compare type — `(unsigned)x - (unsigned)K`,
 * never `(unsigned)(x - K)`. The signed spelling overflows int/long long
 * at the swept domain extremes (INT_MIN, INT_MAX), and a differentially
 * compared oracle must itself be defined C, not just look like the fold.
 */
#include <stdio.h>
#include <limits.h>

#define NOINLINE __attribute__((noinline))

/* ------------------------------------------------------------------ *
 * Positive shapes: the branch form of the range idiom across widths,
 * orientations, and control-flow contexts. Each has a hand oracle.
 * ------------------------------------------------------------------ */

/* P1: the csv_field_sum digit test (signed char domain, loop context). */
NOINLINE static int p_digit(char c) {
    if (c >= '0' && c <= '9') return 1;
    return 0;
}
static int r_digit(char c) { return (int)(unsigned char)(c - '0') <= 9; }

/* P2: plain int range. */
NOINLINE static int p_i32(int x) {
    if (x >= 100 && x <= 200) return 5;
    return 7;
}
static int r_i32(int x) { return (unsigned)x - 100u <= 100u ? 5 : 7; }

/* P3: unsigned compares. */
NOINLINE static int p_u32(unsigned x) {
    if (x >= 4000000000u && x <= 4000000010u) return 1;
    return 0;
}
static int r_u32(unsigned x) { return x - 4000000000u <= 10u; }

/* P4: bounds in the other order. */
NOINLINE static int p_swap(int x) {
    if (x <= 'z' && x >= 'a') return 3;
    return 4;
}
static int r_swap(int x) { return (unsigned)x - (unsigned)'a' <= 25u ? 3 : 4; }

/* P5: constants on the LHS. */
NOINLINE static int p_lhs(int x) {
    if ('A' <= x && x <= 'F') return 1;
    return 0;
}
static int r_lhs(int x) { return (unsigned)x - (unsigned)'A' <= 5u; }

/* P6: the || outside form. */
NOINLINE static int p_or(signed char x) {
    if (x < -3 || x > 3) return 1;
    return 0;
}
static int r_or(signed char x) {
    return (int)(unsigned char)(x - (-3)) > 6;
}

/* P7: the range as a while guard — the loop-latch phi retarget path. */
NOINLINE static int p_while(int x) {
    int n = 0;
    while (x >= 10 && x <= 20) {
        x -= 3;
        n++;
    }
    return n;
}
static int r_while(int x) {
    int n = 0;
    while ((unsigned)x - 10u <= 10u) {
        x -= 3;
        n++;
    }
    return n;
}

/* P8: nested ranges — inner and outer both foldable. */
NOINLINE static int p_nested(int x) {
    if (x >= 0 && x <= 100) {
        if (x >= 10 && x <= 20) return 1;
        return 2;
    }
    return 3;
}
static int r_nested(int x) {
    if ((unsigned)x <= 100u) return (unsigned)x - 10u <= 10u ? 1 : 2;
    return 3;
}

/* P9: 64-bit range. */
NOINLINE static int p_i64(long long x) {
    if (x >= -1000000000000LL && x <= 1000000000000LL) return 1;
    return 0;
}
static int r_i64(long long x) {
    return (unsigned long long)x - (unsigned long long)(-1000000000000LL)
           <= 2000000000000ULL;
}

/* P10: unsigned short range (u16 compare domain). */
NOINLINE static int p_u16(unsigned short x) {
    if (x >= 50000u && x <= 50025u) return 9;
    return 11;
}
static int r_u16(unsigned short x) { return (unsigned short)(x - 50000u) <= 25u ? 9 : 11; }

/* P11: signed byte range across the sign boundary. */
NOINLINE static int p_s8(signed char x) {
    if (x >= -100 && x <= 100) return 1;
    return 0;
}
static int r_s8(signed char x) { return (int)(unsigned char)(x - (-100)) <= 200; }

/* P12: collapsed range (span 0) — an equality test. */
NOINLINE static int p_eq(int x) {
    if (x >= 42 && x <= 42) return 1;
    return 0;
}
static int r_eq(int x) { return (unsigned)x - 42u == 0u; }

/* P13: range guarding a body too big for if-conversion (many statements). */
NOINLINE static int p_bigbody(int x, int *acc) {
    if (x >= -50 && x <= 50) {
        *acc += 1;
        *acc ^= 0x5555;
        *acc += 2;
        *acc ^= 0xaaaa;
        *acc += 3;
        *acc ^= 0x3333;
        *acc += 4;
        *acc ^= 0xcccc;
        *acc += 5;
        *acc ^= 0x7777;
        return 1;
    }
    return 0;
}
static int r_bigbody(int x, int *acc) {
    if ((unsigned)x + 50u > 100u) return 0;
    *acc += 1;
    *acc ^= 0x5555;
    *acc += 2;
    *acc ^= 0xaaaa;
    *acc += 3;
    *acc ^= 0x3333;
    *acc += 4;
    *acc ^= 0xcccc;
    *acc += 5;
    *acc ^= 0x7777;
    return 1;
}

/* ------------------------------------------------------------------ *
 * Adversarial shapes: the fold must reject these (or fold them
 * soundly); either way the results must be exact C semantics.
 * ------------------------------------------------------------------ */

/* A1: the two compares test DIFFERENT values — never a range. */
NOINLINE static int a1_diff_values(int x, int y) {
    if (x >= 10 && y <= 20) return 1;
    return 0;
}
static int a1_ref(int x, int y) { return (x >= 10) & (y <= 20); }

/* A2: mixed signedness across the two compares — different domains. */
NOINLINE static int a2_mixed_sign(int x) {
    if ((unsigned)x >= 48u && x <= 57) return 1;
    return 0;
}
static int a2_ref(int x) { return ((unsigned)x >= 48u) & (x <= 57); }

/* A3: unrepresentable span at the compare width (I32 full range). */
NOINLINE static int a3_span_overflow(int x) {
    if (x >= INT_MIN && x <= INT_MAX) return 1;
    return 0;
}
static int a3_ref(int x) { (void)x; return 1; }

/* A4: the first compare is ALSO consumed as a value — the branch fold
 * must not steal it (the Select fold owns value contexts). */
NOINLINE static int a4_value_use(int c, int *seen) {
    int first = (c >= '0');
    if (c >= '0' && c <= '9') {
        *seen += first;
        return 10 + first;
    }
    return first;
}
static int a4_ref(int c, int *seen) {
    int first = (c >= '0');
    if (first && (c <= '9')) {
        *seen += first;
        return 10 + first;
    }
    return first;
}

/* A5: side effect between the two conditions — never foldable. */
NOINLINE static int a5_side_effect(int c, int *log) {
    if (c >= '0' && (*log = (c <= '9'), 1) && c <= '9') return 1;
    return 0;
}
static int a5_ref(int c, int *log) {
    if (c >= '0') {
        *log = (c <= '9');
        if (c <= '9') return 1;
    }
    return 0;
}

/* A6: the range result feeds a phi that ALSO has an arm from the check
 * block's other path — the merge discipline must hold. */
NOINLINE static int a6_phi_merge(int c) {
    int v = 0;
    if (c >= 'a' && c <= 'z') {
        v = c - 'a' + 1;
    } else {
        v = -1;
    }
    return v;
}
static int a6_ref(int c) {
    if ((unsigned)c - (unsigned)'a' <= 25u) return c - 'a' + 1;
    return -1;
}

/* A7: a second-range chain one arm of which is itself a range — only
 * the inner shape can fold; the outer must stay sound either way. */
NOINLINE static int a7_outer_norangefold(int c) {
    if ((c >= '0' && c <= '9') || (c >= 'A' && c <= 'Z')) return 1;
    return 0;
}
static int a7_ref(int c) {
    return ((unsigned)c - (unsigned)'0' <= 9u) | ((unsigned)c - (unsigned)'A' <= 25u);
}

/* A8: the || form with an unrepresentable span (signed full range). */
NOINLINE static int a8_or_span(int x) {
    if (x < INT_MIN + 1 || x > INT_MAX - 1) return 1;
    return 0;
}
static int a8_ref(int x) { return (x < INT_MIN + 1) | (x > INT_MAX - 1); }

/* A10: unsigned-char bounds exceeding the source domain. [250, 260] is
 * true for 250..255 ONLY; a narrowed `(u8)(x - 250) <= 10` also accepts
 * 0..4 through the wraparound alias (reproduced live: five wrong values
 * per sweep). The fold must keep the compare at the promoted width. */
NOINLINE static int a10_domain(unsigned char x) {
    if (x >= 250 && x <= 260) return 1;
    return 0;
}
static int a10_ref(unsigned char x) { return x >= 250; }

/* A10v: the value-context (Select) form of the same shape — the path the
 * narrowing alias actually fired on. */
NOINLINE static int a10v_domain(unsigned char x) {
    int in = (x >= 250 && x <= 260);
    return in * 7 + (in ? 1 : 0);
}
static int a10v_ref(unsigned char x) { return (x >= 250) * 7 + (x >= 250); }

/* A11: a WRAPPING u64 range is EMPTY, not a 21-value window. The bounds
 * must order in the comparison's own domain — an i64 ordering of the raw
 * bit patterns folds this into a window that wrongly accepts both 0..10
 * and u64max-8..u64max (reproduced live). */
NOINLINE static int a11_wrap_u64(unsigned long long x) {
    if (x >= 18446744073709551607ULL && x <= 10ULL) return 1;
    return 0;
}
static int a11_wrap_u64_ref(unsigned long long x) { (void)x; return 0; }

NOINLINE static int a11v_wrap_u64(unsigned long long x) {
    int in = (x >= 18446744073709551607ULL && x <= 10ULL);
    return in * 3;
}
static int a11v_wrap_u64_ref(unsigned long long x) { (void)x; return 0; }

/* A11u: the same wrapping class at u32 width. */
NOINLINE static int a11_wrap_u32(unsigned x) {
    if (x >= 4294967286u && x <= 10u) return 1;
    return 0;
}
static int a11_wrap_u32_ref(unsigned x) { (void)x; return 0; }

/* A12: signed 64-bit full-range bounds — the span 2^64-1 is not
 * representable, the fold must reject, and the arithmetic must not
 * overflow (a checked build would panic on the i64 span subtraction). */
NOINLINE static int a12_span_i64(long long x) {
    if (x >= LLONG_MIN && x <= LLONG_MAX) return 1;
    return 0;
}
static int a12_ref(long long x) { (void)x; return 1; }

/* ------------------------------------------------------------------ */

static int fails = 0;

#define CHECK(expr, ref, ctx)                                                  \
    do {                                                                       \
        int _e = (expr);                                                       \
        int _r = (ref);                                                        \
        if (_e != _r) {                                                        \
            printf("MISMATCH %s ctx=%d: got %d want %d\n", #expr, (ctx), _e,  \
                   _r);                                                        \
            fails++;                                                           \
        }                                                                      \
    } while (0)

int main(void) {
    unsigned long long h = 1469598103934665603ULL;
    int i;

    /* Byte-domain sweep: every value, plus the signed-char mirror. */
    for (i = 0; i <= 255; i++) {
        char c = (char)(i - 128);
        unsigned char u = (unsigned char)i;
        CHECK(p_digit(c), r_digit(c), i);
        CHECK(p_digit((char)u), r_digit((char)u), i);
        CHECK(p_or(c), r_or(c), i);
        CHECK(p_s8(c), r_s8(c), i);
        CHECK(p_swap(i - 128), r_swap(i - 128), i);
        CHECK(p_lhs(i - 128), r_lhs(i - 128), i);
        CHECK(a10_domain(u), a10_ref(u), i);
        CHECK(a10v_domain(u), a10v_ref(u), i);
        CHECK(a11_wrap_u32((unsigned)i), a11_wrap_u32_ref((unsigned)i), i);
        h = h * 31 + (unsigned)p_digit(c) + (unsigned)p_or(c) * 7
            + (unsigned)a10_domain(u) * 3;
    }

    /* Edge-value sweep for the 32-bit shapes. */
    {
        static const int edges[] = {
            INT_MIN, INT_MIN + 1, -101, -100, -99, -51, -50, -49, -1,  0,
            1,       9,            10,    11,    19,   20,   21,   41,  42, 43,
            47,      48,           49,    57,    58,   64,   65,   70,  90, 96,
            97,      99,           100,   101,   199,  200,  201,  INT_MAX - 1,
            INT_MAX,
        };
        for (i = 0; i < (int)(sizeof(edges) / sizeof(edges[0])); i++) {
            int x = edges[i];
            int seen = 0, seen_ref = 0, log = -1, log_ref = -1;
            CHECK(p_i32(x), r_i32(x), x);
            CHECK(p_swap(x), r_swap(x), x);
            CHECK(p_lhs(x), r_lhs(x), x);
            CHECK(p_while(x), r_while(x), x);
            CHECK(p_nested(x), r_nested(x), x);
            CHECK(p_eq(x), r_eq(x), x);
            CHECK(a1_diff_values(x, x), a1_ref(x, x), x);
            CHECK(a1_diff_values(x, x ^ 1), a1_ref(x, x ^ 1), x);
            CHECK(a2_mixed_sign(x), a2_ref(x), x);
            CHECK(a3_span_overflow(x), a3_ref(x), x);
            CHECK(a4_value_use(x, &seen), a4_ref(x, &seen_ref), x);
            if (seen != seen_ref) {
                printf("MISMATCH a4 seen ctx=%d: %d vs %d\n", x, seen, seen_ref);
                fails++;
            }
            CHECK(a5_side_effect(x, &log), a5_ref(x, &log_ref), x);
            if (log != log_ref) {
                printf("MISMATCH a5 log ctx=%d: %d vs %d\n", x, log, log_ref);
                fails++;
            }
            CHECK(a6_phi_merge(x), a6_ref(x), x);
            CHECK(a7_outer_norangefold(x), a7_ref(x), x);
            CHECK(a8_or_span(x), a8_ref(x), x);
            {
                int acc = 0, acc_ref = 0;
                CHECK(p_bigbody(x, &acc), r_bigbody(x, &acc_ref), x);
                if (acc != acc_ref) {
                    printf("MISMATCH p_bigbody acc ctx=%d: %d vs %d\n", x, acc,
                           acc_ref);
                    fails++;
                }
            }
            h = h * 33 + (unsigned)p_i32(x) + (unsigned)p_while(x) * 5
                + (unsigned)a6_phi_merge(x);
        }
    }

    /* u16 / u32 / i64 sweeps. */
    for (i = 0; i < 4096; i += 13) {
        unsigned short w = (unsigned short)(i * 16);
        CHECK(p_u16(w), r_u16(w), w);
        CHECK(p_u16((unsigned short)(w + 1)), r_u16((unsigned short)(w + 1)), w);
    }
    {
        static const unsigned ue[] = {0u,
                                      1u,
                                      9u,
                                      10u,
                                      11u,
                                      3999999999u,
                                      4000000000u,
                                      4000000005u,
                                      4000000010u,
                                      4000000011u,
                                      4294967285u,
                                      4294967286u,
                                      4294967294u,
                                      4294967295u};
        for (i = 0; i < (int)(sizeof(ue) / sizeof(ue[0])); i++)
            CHECK(p_u32(ue[i]), r_u32(ue[i]), (int)i);
    }
    {
        static const long long le[] = {-1000000000001LL, -1000000000000LL, -1,
                                       0,   1,   999999999999LL,
                                       1000000000000LL, 1000000000001LL,
                                       0x7fffffffffffffffLL};
        for (i = 0; i < (int)(sizeof(le) / sizeof(le[0])); i++) {
            CHECK(p_i64(le[i]), r_i64(le[i]), (int)i);
            CHECK(a12_span_i64(le[i]), a12_ref(le[i]), (int)i);
        }
        /* The wrapped-u64 sweep: both ends of the phantom window. */
        {
            static const unsigned long long we[] = {
                0ULL, 1ULL, 9ULL, 10ULL, 11ULL, 0xFFFFFFFFFFFFFFF5ULL,
                0xFFFFFFFFFFFFFFF6ULL, 0xFFFFFFFFFFFFFFF7ULL,
                0xFFFFFFFFFFFFFFFFULL, 1ULL << 63, (1ULL << 63) + 5ULL};
            for (i = 0; i < (int)(sizeof(we) / sizeof(we[0])); i++) {
                CHECK(a11_wrap_u64(we[i]), a11_wrap_u64_ref(we[i]), (int)i);
                CHECK(a11v_wrap_u64(we[i]), a11v_wrap_u64_ref(we[i]), (int)i);
            }
        }
    }

    /* The while-guard shape over every start value in and around the
     * range (the loop-latch retarget path exercises repeated entries). */
    for (i = 5; i <= 30; i++)
        CHECK(p_while(i), r_while(i), i);

    if (fails) {
        printf("range_fold_branch: %d mismatches\n", fails);
        return 1;
    }
    printf("range_fold_branch ok h=%llu\n", h);
    return 0;
}
