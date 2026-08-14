/*
 * Short-circuit booleans returned from functions and consumed as branch
 * conditions across inlining boundaries.
 *
 * Covers the "last link" of a short-circuit chain: when `a || b || c` is a
 * function's return value, the final link's boolean is materialized (setcc +
 * extend) and must remain a correct 0/1 — while the earlier links fuse into
 * branches. Also exercises boolean negation, double negation, and the
 * return-bool feeding a loop condition.
 */
#include <stdio.h>

static int or3_ret(unsigned char c) { return c < 'a' || c > 'z' || c == 0; }
static int and3_ret(int a, int b, int c) { return a && b && c; }
static int neg_or(int a, int b) { return !(a || b); }
static int notnot(int a) { return !!a; }
static int mix_ret(int a, int b, int c) { return (a && b) || c; }
static int cmp_ret(int x) { return x < 100; }

static int loop_with_ret(int n) {
    int i, acc = 0;
    for (i = 0; i < n; i++)
        if (cmp_ret(i) || and3_ret(i & 1, i & 2, 1))
            acc += i;
    return acc;
}

static unsigned long long mix(unsigned long long h, unsigned long long v) {
    return (h ^ v) * 0x9e3779b97f4a7c15ULL + 0xdeadbeefULL;
}

int main(void) {
    unsigned long long h = 1469598103934665603ULL;
    int c;
    for (c = 0; c <= 255; c++) {
        h = mix(h, (unsigned)or3_ret((unsigned char)c));
        h = mix(h, (unsigned)neg_or(c & 1, c & 2));
        h = mix(h, (unsigned)notnot(c));
        h = mix(h, (unsigned)cmp_ret(c - 128));
    }
    h = mix(h, (unsigned)and3_ret(0, 0, 0));
    h = mix(h, (unsigned)and3_ret(1, 1, 1));
    h = mix(h, (unsigned)mix_ret(0, 0, 1));
    h = mix(h, (unsigned)mix_ret(1, 1, 0));
    h = mix(h, (unsigned)loop_with_ret(500));
    printf("%llu\n", (unsigned long long)h);
    return 0;
}
