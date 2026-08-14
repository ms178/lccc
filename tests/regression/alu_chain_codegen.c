/*
 * GPR simple-ALU LHS register hints and the fused
 * compare-to-branch chain (v9/v11). Behavioral coverage of the patterns
 * those codegen paths rewrite, over wrap-sensitive edge values:
 *   x - lo (range bias), x + y + z, x & m | o, x ^ c, x * k + y chains,
 *   and the Cmp -> Cast -> branch shape in loop conditions.
 */
#include <stdio.h>
#include <limits.h>

static unsigned bias(unsigned char c) { return (unsigned)c - 97; }
static int chainadd(int a, int b, int c) { return a + b + c; }
static int chainsub(int a, int b, int c) { return a - b - c; }
static int chainmix(int a, int b, int c) { return (a + b) * (c - a); }
static unsigned maskor(unsigned x) { return (x & 0xff00u) | 0x55u; }
static unsigned maskxor(unsigned x) { return (x & 0xffffu) ^ 0xaaaa; }

static int loop_cond(int n, int step) {
    int i, acc = 0;
    for (i = 0; i < n; i += step) acc += i & 7;
    return acc;
}
static int nested_cond(int n) {
    int i, j, acc = 0;
    for (i = 0; i < n; i++)
        for (j = i; j < n; j++)
            acc += (i ^ j) & 3;
    return acc;
}

static unsigned long long mix(unsigned long long h, unsigned long long v) {
    return (h ^ v) * 0x9e3779b97f4a7c15ULL + 0xdeadbeefULL;
}

int main(void) {
    unsigned long long h = 1469598103934665603ULL;
    int c;
    for (c = 0; c <= 255; c++) {
        h = mix(h, (unsigned)bias((unsigned char)c));
        h = mix(h, (unsigned)chainadd(c, c - 5, c * 3));
        h = mix(h, (unsigned)chainsub(c, c - 5, 3));
        h = mix(h, (unsigned)chainmix(c, c - 5, c + 7));
        h = mix(h, (unsigned)maskor((unsigned)c * 257));
        h = mix(h, (unsigned)maskxor((unsigned)c * 257));
    }
    h = mix(h, (unsigned)loop_cond(1000, 3));
    h = mix(h, (unsigned)loop_cond(1000, 7));
    h = mix(h, (unsigned)nested_cond(60));
    {
        static const int xs[] = { INT_MIN, INT_MIN + 1, -1, 0, 1, 2, INT_MAX - 1, INT_MAX };
        unsigned n;
        for (n = 0; n < sizeof(xs) / sizeof(xs[0]); n++) {
            h = mix(h, (unsigned)chainadd(xs[n], 5, -3));
            h = mix(h, (unsigned)chainsub(xs[n], -5, 3));
        }
    }
    printf("%llu\n", (unsigned long long)h);
    return 0;
}
