/*
 * GPR unary producer->consumer hints (Neg / Not / Bswap).
 *
 * These unary ops compute into the destination register with the source
 * pre-loaded (`neg %reg`, `not %reg`, `bswap %reg`), so the allocator may
 * share the producer's register at die-at-birth. Covers the edge values that
 * exercise the wraparound (INT_MIN negation, all byte patterns for bswap).
 */
#include <stdio.h>
#include <limits.h>

static int neg(int x) { return -x; }
static unsigned not_(unsigned x) { return ~x; }
static unsigned bswap32(unsigned x) { return __builtin_bswap32(x); }
static unsigned long long bswap64(unsigned long long x) { return __builtin_bswap64(x); }
static unsigned short bswap16(unsigned short x) { return __builtin_bswap16(x); }

/* Unary chains: the neg result feeds another neg / not. */
static int neg_neg(int x) { return -(-x); }
static int neg_plus(int x) { return -x + x; }
static unsigned not_not(unsigned x) { return ~~x; }

static unsigned long long mix(unsigned long long h, unsigned long long v) {
    return (h ^ v) * 0x9e3779b97f4a7c15ULL + 0xdeadbeefULL;
}

int main(void) {
    unsigned long long h = 1469598103934665603ULL;
    {
        static const int vals[] = { INT_MIN, INT_MIN + 1, -1, 0, 1, 2, 1234567, INT_MAX - 1, INT_MAX };
        unsigned i;
        for (i = 0; i < sizeof(vals) / sizeof(vals[0]); i++) {
            h = mix(h, (unsigned)neg(vals[i]));
            h = mix(h, (unsigned)neg_neg(vals[i]));
            h = mix(h, (unsigned)neg_plus(vals[i]));
            h = mix(h, (unsigned)not_(vals[i]));
            h = mix(h, (unsigned)not_not(vals[i]));
            h = mix(h, (unsigned)bswap32((unsigned)vals[i]));
        }
    }
    {
        unsigned x;
        for (x = 0; x < 4096; x++) {
            unsigned v = x * 2654435761u;
            h = mix(h, (unsigned)bswap16((unsigned short)v));
            h = mix(h, (unsigned)bswap32(v));
            h = mix(h, (unsigned long long)bswap64((unsigned long long)v << 32 | v));
        }
    }
    printf("%llu\n", (unsigned long long)h);
    return 0;
}
