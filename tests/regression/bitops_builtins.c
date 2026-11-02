/*
 * bit-manipulation builtins (popcount, clz, ctz, ffs,
 * parity, bswap) over edge values, including the undefined-input cases the
 * codegen must handle deterministically (clz/ctz of 0, parity of 0).
 * Differential vs GCC.
 */
#include <stdio.h>
#include <limits.h>

static int pc(unsigned x) { return __builtin_popcount(x); }
static int pcl(unsigned long x) { return __builtin_popcountl(x); }
static int pcll(unsigned long long x) { return __builtin_popcountll(x); }
static int clz(unsigned x) { return __builtin_clz(x); }
static int ctz(unsigned x) { return __builtin_ctz(x); }
static int ffs_(int x) { return __builtin_ffs(x); }
static int par(unsigned x) { return __builtin_parity(x); }
static unsigned b16(unsigned short x) { return __builtin_bswap16(x); }
static unsigned b32(unsigned x) { return __builtin_bswap32(x); }
static unsigned long long b64(unsigned long long x) { return __builtin_bswap64(x); }

static unsigned long long mix(unsigned long long h, unsigned long long v) {
    return (h ^ v) * 0x9e3779b97f4a7c15ULL + 0xdeadbeefULL;
}

int main(void) {
    unsigned long long h = 1469598103934665603ULL;
    {
        static const unsigned xs[] = { 0, 1, 2, 3, 4, 0x7fffffff, 0x80000000, 0x80000001,
            0xaaaaaaaa, 0x55555555, 0xffffffff };
        unsigned n;
        for (n = 0; n < sizeof(xs) / sizeof(xs[0]); n++) {
            h = mix(h, (unsigned)pc(xs[n]));
            h = mix(h, (unsigned)clz(xs[n]));
            h = mix(h, (unsigned)ctz(xs[n]));
            h = mix(h, (unsigned)ffs_((int)xs[n]));
            h = mix(h, (unsigned)par(xs[n]));
            h = mix(h, (unsigned)b32(xs[n]));
        }
        static const unsigned long long ys[] = { 0, 1, 0x8000000000000000ULL,
            0xffffffffffffffffULL, 0x123456789abcdef0ULL, 0x00ff00ff00ff00ffULL };
        for (n = 0; n < sizeof(ys) / sizeof(ys[0]); n++) {
            h = mix(h, (unsigned long long)pcll(ys[n]));
            h = mix(h, (unsigned long long)b64(ys[n]));
        }
    }
    unsigned short s;
    for (s = 0; s < 4096; s += 53)
        h = mix(h, (unsigned)b16(s));
    printf("%llu\n", (unsigned long long)h);
    return 0;
}
