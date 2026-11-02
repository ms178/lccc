/* Shared body of the i686_scalar_fp_isa_*.c fixtures: scalar F32
 * arithmetic, F32 compares and every float<->int conversion width, under
 * all four rounding directions, compiled per ISA level by the including
 * file's .flags (pre-SSE x87, SSE1-only, the default -m32 SSE2 baseline
 * without SSE3, and SSE3).
 *
 * GCC built with the same flags is the stdout oracle. Every operation is a
 * single IEEE operation between volatile objects, so x87 excess precision
 * cannot leak into a result (the extended intermediate of one +,-,*,/ is
 * innocuous: 64 >= 2*24+2 significand bits), and nothing constant-folds
 * across fesetround. Conversions stay inside their defined domain, except
 * F32/F64 -> I32 at the indefinite boundary, where both the x87 32-bit
 * store and CVTT*2SI yield 0x80000000.
 *
 * After each conversion batch the rounding direction must be the one that
 * was set: the no-SSE3 truncating store switches the x87 control word and
 * must restore it, which fegetround() and a directed x87 division observe.
 */
#include <fenv.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

static uint32_t fbits(float f) {
    uint32_t u;
    memcpy(&u, &f, 4);
    return u;
}
static float bitsf(uint32_t u) {
    float f;
    memcpy(&f, &u, 4);
    return f;
}
static uint64_t dbits(double d) {
    uint64_t u;
    memcpy(&u, &d, 8);
    return u;
}

static uint64_t h = 1469598103934665603ull;
static void mix(uint64_t v) {
    for (int i = 0; i < 8; i++) {
        h ^= (v >> (8 * i)) & 0xff;
        h *= 1099511628211ull;
    }
}

static const uint32_t fvals[] = {
    0x00000000u, 0x80000000u,             /* +-0 */
    0x3f800000u, 0xbf800000u,             /* +-1 */
    0x3f000000u, 0x3fc00000u, 0x40200000u, /* 0.5 1.5 2.5 */
    0xc0200000u, 0x407fffffu,             /* -2.5, 3.9999998 */
    0x7f7fffffu, 0xff7fffffu,             /* +-FLT_MAX */
    0x00800000u, 0x80800000u,             /* +-FLT_MIN */
    0x00000001u, 0x807fffffu,             /* subnormal min, -max subnormal */
    0x7f800000u, 0xff800000u,             /* +-inf */
    0x7fc00000u,                          /* qNaN */
    0x3dcccccdu, 0x3eaaaaabu,             /* 0.1 1/3 */
    0x4effffffu, 0x4f000000u,             /* 2147483520, 2^31 */
    0x4f7fffffu, 0xcf000000u,             /* 4294967040, -2^31 */
    0x4b800001u, 0x33800000u,             /* 16777218, 2^-24 */
};
#define NF (sizeof fvals / sizeof fvals[0])

static void f32_arith_and_compare(void) {
    for (unsigned i = 0; i < NF; i++) {
        for (unsigned j = 0; j < NF; j++) {
            volatile float a = bitsf(fvals[i]), b = bitsf(fvals[j]);
            volatile float s = a + b, d = a - b, m = a * b, q = a / b;
            mix(fbits(s));
            mix(fbits(d));
            mix(fbits(m));
            mix(fbits(q));
            unsigned c = (a < b) | (a <= b) << 1 | (a > b) << 2 | (a >= b) << 3 |
                         (a == b) << 4 | (a != b) << 5;
            mix(c);
        }
    }
}

static int in_open(double v, double lo, double hi) { return v > lo && v < hi; }

static void conversions(void) {
    /* F32 -> integers. */
    for (unsigned i = 0; i < NF; i++) {
        volatile float f = bitsf(fvals[i]);
        double v = f;
        if (v != v)
            continue;
        if (in_open(v, -2147483649.0, 2147483648.0)) {
            volatile int r = (int)f;
            mix((uint32_t)r);
        }
        if (in_open(v, -1.0, 4294967296.0)) {
            volatile unsigned r = (unsigned)f;
            mix(r);
        }
        if (in_open(v, -32769.0, 32768.0)) {
            volatile short r = (short)f;
            mix((uint16_t)r);
        }
        if (in_open(v, -1.0, 65536.0)) {
            volatile unsigned short r = (unsigned short)f;
            mix(r);
        }
        if (in_open(v, -129.0, 128.0)) {
            volatile signed char r = (signed char)f;
            mix((uint8_t)r);
        }
        if (in_open(v, -9223372036854775808.0 - 1.0, 9223372036854775808.0)) {
            volatile long long r = (long long)f;
            mix((uint64_t)r);
        }
        if (in_open(v, -1.0, 18446744073709551616.0)) {
            volatile unsigned long long r = (unsigned long long)f;
            mix(r);
        }
    }
    /* The I32 indefinite boundary (identical for x87 and CVTTSS2SI). */
    {
        volatile float big = 2147483648.0f;
        volatile int r = (int)big;
        mix((uint32_t)r);
    }
    /* F64 -> integers. */
    static const double dvals[] = {
        0.0, -0.0, 0.9999999999, -0.9999999999, 1.5, -1.5, 2.5, -2.5,
        2147483647.9, -2147483648.9, 4294967295.9, 65535.9, -32768.9, 127.9,
        -128.9, 1e18, -1e18, 9223372036854774784.0, 9223372036854775808.0,
        18446744073709549568.0, 4503599627370497.5, 0x1p-1074,
    };
    for (unsigned i = 0; i < sizeof dvals / sizeof dvals[0]; i++) {
        volatile double d = dvals[i];
        double v = d;
        if (in_open(v, -2147483649.0, 2147483648.0)) {
            volatile int r = (int)d;
            mix((uint32_t)r);
        }
        if (in_open(v, -1.0, 4294967296.0)) {
            volatile unsigned r = (unsigned)d;
            mix(r);
        }
        if (in_open(v, -32769.0, 32768.0)) {
            volatile short r = (short)d;
            mix((uint16_t)r);
        }
        if (in_open(v, -1.0, 65536.0)) {
            volatile unsigned short r = (unsigned short)d;
            mix(r);
        }
        if (in_open(v, -129.0, 128.0)) {
            volatile signed char r = (signed char)d;
            volatile unsigned char u = in_open(v, -1.0, 256.0) ? (unsigned char)d : 0;
            mix((uint8_t)r);
            mix(u);
        }
        if (in_open(v, -9223372036854775808.0 - 1.0, 9223372036854775808.0)) {
            volatile long long r = (long long)d;
            mix((uint64_t)r);
        }
        if (in_open(v, -1.0, 18446744073709551616.0)) {
            volatile unsigned long long r = (unsigned long long)d;
            mix(r);
        }
    }
    /* Integers -> F32 (directed rounding applies to inexact ones). */
    static const int ivals[] = {
        0, 1, -1, 16777216, 16777217, -16777217, 2147483647, -2147483647 - 1,
        0x7fffffc0, 0x7fffffbf, 123456789, -123456789,
    };
    for (unsigned i = 0; i < sizeof ivals / sizeof ivals[0]; i++) {
        volatile int x = ivals[i];
        volatile float f = (float)x;
        volatile short sx = (short)x;
        volatile float fs = (float)sx;
        volatile unsigned short ux = (unsigned short)x;
        volatile float fu = (float)ux;
        mix(fbits(f));
        mix(fbits(fs));
        mix(fbits(fu));
    }
    static const unsigned uvals[] = {
        0u, 1u, 16777217u, 0x7fffffffu, 0x80000000u, 0x80000001u,
        0xffffff7fu, 0xffffff80u, 0xffffffffu, 0xfffffe01u,
    };
    for (unsigned i = 0; i < sizeof uvals / sizeof uvals[0]; i++) {
        volatile unsigned x = uvals[i];
        volatile float f = (float)x;
        mix(fbits(f));
    }
}

int main(void) {
    static const int modes[] = {FE_TONEAREST, FE_UPWARD, FE_DOWNWARD, FE_TOWARDZERO};
    static const char *names[] = {"nearest", "upward", "downward", "towardzero"};
    int failures = 0;
    uint64_t thirds[4];
    uint32_t fthirds[4];
    for (unsigned k = 0; k < 4; k++) {
        if (fesetround(modes[k]) != 0) {
            printf("fesetround(%s) failed\n", names[k]);
            return 1;
        }
        h = 1469598103934665603ull;
        f32_arith_and_compare();
        uint64_t arith = h;
        h = 1469598103934665603ull;
        conversions();
        uint64_t conv = h;
        /* The direction set above must have survived every conversion. */
        int now = fegetround();
        volatile double one = 1.0, three = 3.0;
        volatile double third = one / three;
        volatile float fone = 1.0f, fthree = 3.0f;
        volatile float fthird = fone / fthree;
        thirds[k] = dbits(third);
        fthirds[k] = fbits(fthird);
        if (now != modes[k]) {
            printf("%s: rounding mode clobbered (fegetround=%d)\n", names[k], now);
            failures++;
        }
        printf("%s arith=%016llx conv=%016llx third=%016llx fthird=%08x\n", names[k],
               (unsigned long long)arith, (unsigned long long)conv,
               (unsigned long long)dbits(third), (unsigned)fbits(fthird));
    }
    /* Oracle-independent: a control word left at round-toward-zero by a
     * conversion would make every direction truncate. Exact 1/3 per
     * direction: F64 nearest rounds down, F32 nearest rounds up. */
    static const uint64_t want64[] = {0x3fd5555555555555ull, 0x3fd5555555555556ull,
                                      0x3fd5555555555555ull, 0x3fd5555555555555ull};
    static const uint32_t want32[] = {0x3eaaaaabu, 0x3eaaaaabu, 0x3eaaaaaau, 0x3eaaaaaau};
    fesetround(FE_TONEAREST);
    for (unsigned k = 0; k < 4; k++) {
        uint64_t want = want64[k];
        uint32_t fwant = want32[k];
        if (thirds[k] != want || fthirds[k] != fwant) {
            printf("%s: 1/3 not rounded in its direction\n", names[k]);
            failures++;
        }
    }
    return failures ? 1 : 0;
}
