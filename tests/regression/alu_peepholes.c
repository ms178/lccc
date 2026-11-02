/*
 * ALU peepholes and strength reduction.
 *
 * Exercised patterns (behavioral, differential vs GCC): LEA strength
 * reduction for x*3/x*5/x*9, mul/div/mod by powers of two, division and
 * modulo by constants (signed + unsigned, positive + negative divisors),
 * x*1/x*0/x+0/x-0/x-x/x&0/x|0/x^x identities, and the mul-add fusion shape.
 * Edge values include INT_MIN/INT_MAX and every byte.
 */
#include <stdio.h>
#include <limits.h>

static int mul3(int x) { return x * 3; }
static int mul5(int x) { return x * 5; }
static int mul9(int x) { return x * 9; }
static int mul7(int x) { return x * 7; }        /* not a lea form */
static int mul1(int x) { return x * 1; }
static int mul0(int x) { return x * 0; }
static int add0(int x) { return x + 0; }
static int sub0(int x) { return x - 0; }
static int subself(int x) { return x - x; }
static int and0(int x) { return x & 0; }
static int andall(int x) { return x & -1; }
static int or0(int x) { return x | 0; }
static int xor0(int x) { return x ^ 0; }
static int xorx(int x) { return x ^ x; }

static int sdiv3(int x) { return x / 3; }
static int sdivm3(int x) { return x / -3; }
static int srem3(int x) { return x % 3; }
static unsigned udiv3(unsigned x) { return x / 3; }
static unsigned urem3(unsigned x) { return x % 3; }
static unsigned udiv5(unsigned x) { return x / 5; }
static unsigned udiv17(unsigned x) { return x / 17; }
static int sdiv10(int x) { return x / 10; }
static int sdivm10(int x) { return x / -10; }
static unsigned long long udiv7ull(unsigned long long x) { return x / 7; }

/* mul-add fusion shape */
static int fma_shape(int x, int y) { return x * 3 + y; }

static unsigned long long mix(unsigned long long h, unsigned long long v) {
    return (h ^ v) * 0x9e3779b97f4a7c15ULL + 0xdeadbeefULL;
}

int main(void) {
    unsigned long long h = 1469598103934665603ULL;
    int c;
    for (c = 0; c <= 255; c++) {
        int v = c - 128;
        h = mix(h, (unsigned)mul3(v));
        h = mix(h, (unsigned)mul5(v));
        h = mix(h, (unsigned)mul9(v));
        h = mix(h, (unsigned)mul7(v));
        h = mix(h, (unsigned)mul1(v));
        h = mix(h, (unsigned)mul0(v));
        h = mix(h, (unsigned)add0(v));
        h = mix(h, (unsigned)sub0(v));
        h = mix(h, (unsigned)subself(v));
        h = mix(h, (unsigned)and0(v));
        h = mix(h, (unsigned)andall(v));
        h = mix(h, (unsigned)or0(v));
        h = mix(h, (unsigned)xor0(v));
        h = mix(h, (unsigned)xorx(v));
        h = mix(h, (unsigned)sdiv3(v));
        h = mix(h, (unsigned)sdivm3(v));
        h = mix(h, (unsigned)srem3(v));
        h = mix(h, (unsigned)udiv3((unsigned)v));
        h = mix(h, (unsigned)urem3((unsigned)v));
        h = mix(h, (unsigned)udiv5((unsigned)v));
        h = mix(h, (unsigned)udiv17((unsigned)v));
        h = mix(h, (unsigned)sdiv10(v));
        h = mix(h, (unsigned)sdivm10(v));
        h = mix(h, (unsigned)fma_shape(v, v + 1));
    }
    {
        static const int xs[] = { INT_MIN, INT_MIN + 1, -1000003, -1, 0, 1, 2, 999983, INT_MAX - 1, INT_MAX };
        unsigned n;
        for (n = 0; n < sizeof(xs) / sizeof(xs[0]); n++) {
            h = mix(h, (unsigned)sdiv3(xs[n]));
            h = mix(h, (unsigned)sdivm3(xs[n]));
            h = mix(h, (unsigned)srem3(xs[n]));
            h = mix(h, (unsigned)sdiv10(xs[n]));
            h = mix(h, (unsigned)sdivm10(xs[n]));
            h = mix(h, (unsigned)mul3(xs[n]));
        }
        static const unsigned long long ys[] = { 0, 1, 7, 0xffffffffffffffffULL, 0x8000000000000000ULL, 1234567890123456789ULL };
        for (n = 0; n < sizeof(ys) / sizeof(ys[0]); n++)
            h = mix(h, (unsigned long long)udiv7ull(ys[n]));
    }
    printf("%llu\n", (unsigned long long)h);
    return 0;
}
