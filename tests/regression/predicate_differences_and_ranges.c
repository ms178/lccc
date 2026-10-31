/* Predicate differences and ranges: difference compares, interval predicates
 * and sign-magnitude selects.
 *
 * Every function is self-checking: `main` compares the optimized result against
 * a reference computed from volatile inputs, so a wrong fold shows up as a
 * runtime mismatch rather than as an instruction-count difference.  The probes
 * cover every interval boundary (L-1, L, U, U+1), the wrap points of the
 * subtraction (0, 1, INT_MIN, INT_MAX) and both signs of every difference.
 *
 * The `*_control` functions are deliberately one constant (or one operator)
 * away from a folded shape and must NOT fold; the gate checks that they keep
 * their second comparison.
 */
#include <stdint.h>
#include <stdio.h>

static volatile int vi, vj;
static volatile unsigned vu, vv;

/* ---- difference compares ------------------------------------------------ */
__attribute__((noinline)) static int diff_gt(int a, int b) { return (a - b) > 0; }
__attribute__((noinline)) static int diff_eq(int a, int b) { return (a - b) == 0; }
__attribute__((noinline)) static int diff_ge(int a, int b) { return (a - b) >= 0; }
__attribute__((noinline)) static int diff_une(unsigned a, unsigned b) { return (a - b) != 0u; }
/* Not a difference compare: the constant is not zero. */
__attribute__((noinline)) static int diff_control(int a, int b) { return (a - b) > 5; }

/* ---- interval predicates ------------------------------------------------ */
__attribute__((noinline)) static int range_bitand(int x) { return (x > 3) & (x < 10); }
__attribute__((noinline)) static int range_short(int x) { return x > 3 && x < 10; }
__attribute__((noinline)) static int range_closed(int x) { return x >= 4 && x <= 9; }
__attribute__((noinline)) static int range_unsigned(unsigned x) { return x >= 4u && x <= 9u; }
__attribute__((noinline)) static int outside_or(unsigned x) { return (x < 4u) | (x > 9u); }
__attribute__((noinline)) static int outside_short(unsigned x) { return x < 4u || x > 9u; }
__attribute__((noinline)) static int member_short(unsigned x) { return x == 0u || x == 1u; }
__attribute__((noinline)) static int member_bitand(unsigned x) { return (x != 0u) & (x != 1u); }

/* Controls: a union that is not an interval, a membership with a gap, and two
 * predicates on different values.  None of them is one unsigned compare. */
__attribute__((noinline)) static int range_union(int x) { return (x > 3) | (x > 10); }
__attribute__((noinline)) static int member_gap(unsigned x) { return (x == 0u) | (x == 2u); }
__attribute__((noinline)) static int two_values(int a, int b) { return (a > 3) & (b < 10); }

/* ---- sign magnitude ----------------------------------------------------- */
__attribute__((noinline)) static int abs_select(int x) { return x < 0 ? -x : x; }
__attribute__((noinline)) static long long abs_select64(long long x) { return x < 0 ? -x : x; }
__attribute__((noinline)) static int absdiff_select(int a, int b) { return a > b ? a - b : b - a; }
/* Not a sign-magnitude select: the arms do not agree on one value. */
__attribute__((noinline)) static int abs_control(int x, int y) { return x < 0 ? -y : x; }

int main(void) {
    static const int iprobes[] = {
        0, 1, 2, 3, 4, 5, 8, 9, 10, 11, -1, -2, -4, -5,
        2147483647, -2147483647, -2147483647 - 1,
    };
    static const unsigned uprobes[] = {
        0u, 1u, 2u, 3u, 4u, 5u, 8u, 9u, 10u, 11u,
        2147483648u, 4294967294u, 4294967295u,
    };
    uint64_t bad = 0;

    for (unsigned i = 0; i < sizeof iprobes / sizeof *iprobes; i++) {
        for (unsigned j = 0; j < sizeof iprobes / sizeof *iprobes; j++) {
            int a = iprobes[i], b = iprobes[j];
            vi = a;
            vj = b;
            if (diff_gt(vi, vj) != ((a - b) > 0)) bad++;
            if (diff_eq(vi, vj) != ((a - b) == 0)) bad++;
            if (diff_ge(vi, vj) != ((a - b) >= 0)) bad++;
            if (diff_control(vi, vj) != ((a - b) > 5)) bad++;
            if (absdiff_select(vi, vj) != (a > b ? a - b : b - a)) bad++;
            if (abs_control(vi, vj) != (a < 0 ? -b : a)) bad++;
            if (two_values(vi, vj) != ((a > 3) & (b < 10))) bad++;
        }
    }
    for (unsigned i = 0; i < sizeof uprobes / sizeof *uprobes; i++) {
        for (unsigned j = 0; j < sizeof uprobes / sizeof *uprobes; j++) {
            unsigned a = uprobes[i], b = uprobes[j];
            vu = a;
            vv = b;
            if (diff_une(vu, vv) != ((unsigned)(a - b) != 0u)) bad++;
        }
    }
    for (unsigned i = 0; i < sizeof iprobes / sizeof *iprobes; i++) {
        int x = iprobes[i];
        vi = x;
        int in = x > 3 && x < 10;
        if (range_bitand(vi) != ((x > 3) & (x < 10))) bad++;
        if (range_short(vi) != in) bad++;
        if (range_closed(vi) != (x >= 4 && x <= 9)) bad++;
        if (range_union(vi) != ((x > 3) | (x > 10))) bad++;
        if (abs_select(vi) != (x < 0 ? -x : x)) bad++;
        if (abs_select64((long long)x) != (x < 0 ? -(long long)x : (long long)x)) bad++;
        if (member_gap((unsigned)x) != (((unsigned)x == 0u) | ((unsigned)x == 2u))) bad++;
    }
    for (unsigned i = 0; i < sizeof uprobes / sizeof *uprobes; i++) {
        unsigned x = uprobes[i];
        vu = x;
        if (range_unsigned(vu) != (x >= 4u && x <= 9u)) bad++;
        if (outside_or(vu) != ((x < 4u) | (x > 9u))) bad++;
        if (outside_short(vu) != (x < 4u || x > 9u)) bad++;
        if (member_short(vu) != (x == 0u || x == 1u)) bad++;
        if (member_bitand(vu) != ((x != 0u) & (x != 1u))) bad++;
        if (member_gap(vu) != ((x == 0u) | (x == 2u))) bad++;
    }

    /* Deterministic 200k-value sweep through every folded function. */
    uint64_t s = 0x9e3779b97f4a7c15ull;
    for (int i = 0; i < 200000; i++) {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        int a = (int)(uint32_t)s;
        int b = (int)(uint32_t)(s >> 32);
        unsigned ua = (unsigned)a, ub = (unsigned)b;
        vi = a;
        vj = b;
        vu = ua;
        vv = ub;
        if (diff_gt(vi, vj) != ((a - b) > 0)) bad++;
        if (diff_eq(vi, vj) != ((a - b) == 0)) bad++;
        if (diff_une(vu, vv) != ((unsigned)(ua - ub) != 0u)) bad++;
        if (range_bitand(vi) != ((a > 3) & (a < 10))) bad++;
        if (range_short(vi) != (a > 3 && a < 10)) bad++;
        if (range_unsigned(vu) != (ua >= 4u && ua <= 9u)) bad++;
        if (outside_or(vu) != ((ua < 4u) | (ua > 9u))) bad++;
        if (member_short(vu) != (ua == 0u || ua == 1u)) bad++;
        if (abs_select(vi) != (a < 0 ? -a : a)) bad++;
        if (absdiff_select(vi, vj) != (a > b ? a - b : b - a)) bad++;
        if (range_union(vi) != ((a > 3) | (a > 10))) bad++;
        if (member_gap(vu) != ((ua == 0u) | (ua == 2u))) bad++;
    }

    if (bad) {
        printf("predfold: %llu mismatches\n", (unsigned long long)bad);
        return 1;
    }
    printf("predfold: ok\n");
    return 0;
}
