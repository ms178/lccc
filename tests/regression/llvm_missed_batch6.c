/* Reduced C witnesses for the ten-report LLVM batch-six cohort.
 * All arithmetic that can wrap is unsigned. Volatile reference operands stop
 * identity-based production folds from optimizing both sides of the oracle. */
#include <stdint.h>

#define NOINLINE __attribute__((noinline))
NOINLINE unsigned b6_bits(unsigned x, unsigned object) {
    return ((object == 0) | (x != 13)) & ((x & ~1u) == 42);
}
NOINLINE unsigned b6_sum(unsigned x, unsigned y, unsigned z) {
    return (x == 0) & ((x + y) == z);
}
NOINLINE unsigned b6_range(unsigned x) {
    return ((x + 1) < 3) & ((x & 0x80000001u) == 0x80000001u);
}
NOINLINE unsigned b6_shared(unsigned x, unsigned *out) {
    unsigned r = (x % 6) * 2;
    *out = r;
    return (x / 6) * 12 + r;
}
NOINLINE unsigned b6_masked(unsigned x) {
    return (x >> 4) * 24576 + (((x & 15) * 1536) & 30720);
}
NOINLINE unsigned b6_select(unsigned x, unsigned y) {
    return ((x == 1) & (y == 0)) ? y != x : x > y;
}
NOINLINE unsigned b6_sub(unsigned x, unsigned y) { return (y == 0) & ((x - y) == 1); }
NOINLINE unsigned b6_even(uint64_t x) { return ((x & 1) == 0) & ((x - 2) == 0); }
NOINLINE unsigned b6_signed_select(int x) { return x < 0 ? 0 : x == 0; }
static volatile unsigned observed;
NOINLINE static void observe(unsigned x) { observed ^= x; }
NOINLINE unsigned b6_inverse(unsigned char base, unsigned char offset) {
    unsigned char adjusted = base + offset;
    observe(adjusted);
    _Bool nonzero = adjusted != 0;
    observe(nonzero);
    _Bool no_underflow = adjusted <= base;
    observe(no_underflow);
    unsigned char duplicate = base + offset;
    return !(duplicate == 0) & no_underflow;
}
NOINLINE uint64_t b6_trip(uint64_t x) { return (((x + 12) >> 2) + 1) & 3; }
NOINLINE unsigned b6_guard(unsigned x) {
    if (x < 13) return (x & 16) == 0 ? 1 : 2;
    return 0;
}
NOINLINE unsigned b6_lowzero(unsigned x) { return (x & (0u - x)) == 0; }
NOINLINE unsigned b6_lowone(unsigned x) { return (x & (0u - x)) == 1; }
NOINLINE unsigned b6_boolor(uint64_t x) { return ((x != 0) | x) != 0; }

/* Negative controls: distinct semantics must remain distinct. */
NOINLINE unsigned b6_trip_near(uint64_t x) { return (((x + 11) >> 2) + 1) & 3; }
NOINLINE unsigned b6_mask_near(unsigned x) { return (x >> 4) * 24576 + (((x & 15) * 1536) & 28672); }
NOINLINE unsigned b6_select_near(unsigned x) { return x ? 2 : 3; }

#ifndef BATCH6_BENCH
int main(void) {
    uint64_t state = UINT64_C(0x3141592653589793);
    for (unsigned i = 0; i < 65536; ++i) {
        state ^= state << 13; state ^= state >> 7; state ^= state << 17;
        volatile unsigned x = i < 32768 ? i : (unsigned)state;
        volatile unsigned y = (unsigned)(state >> 32);
        volatile unsigned z = i & 1 ? y : y + 1;
        volatile uint64_t wide = i & 1 ? UINT64_MAX - i : state;
        unsigned out = 0;
        if (b6_bits(x,y) != (((int)(y == 0) | (x != 13)) & ((x & ~1u) == 42))) return 1;
        if (b6_sum(x,y,z) != ((int)(x == 0) & ((x + y) == z))) return 2;
        if (b6_range(x) != ((int)((x + 1) < 3) & ((x & 0x80000001u) == 0x80000001u))) return 3;
        if (b6_shared(x,&out) != (x / 6) * 12 + (x % 6) * 2 || out != (x % 6) * 2) return 4;
        if (b6_masked(x) != (x >> 4) * 24576 + (((x & 15) * 1536) & 30720)) return 5;
        if (b6_select(x,y) != (((int)(x == 1) & (y == 0)) ? y != x : x > y)) return 6;
        unsigned char base = (unsigned char)x, offset = (unsigned char)y;
        unsigned char adjusted = base + offset;
        if (b6_inverse(base,offset) != ((adjusted != 0) & (adjusted <= base))) return 7;
        if (b6_trip(wide) != (((wide + 12) >> 2) + 1) % 4) return 8;
        if (b6_guard(x) != (x < 13 ? ((x & 16) == 0 ? 1u : 2u) : 0u)) return 9;
        if (b6_lowzero(x) != ((x & (0u - x)) == 0)) return 10;
        if (b6_lowone(x) != ((x & (0u - x)) == 1)) return 11;
        if (b6_boolor(wide) != (((wide != 0) | wide) != 0)) return 12;
        if (b6_trip_near(wide) != (((wide + 11) >> 2) + 1) % 4) return 13;
        if (b6_mask_near(x) != (x >> 4) * 24576 + (((x & 15) * 1536) & 28672)) return 14;
        if (b6_select_near(x) != (x ? 2u : 3u)) return 15;
        if (b6_sub(x,y) != ((int)(y == 0) & ((x - y) == 1))) return 16;
        if (b6_even(wide) != ((int)((wide & 1) == 0) & ((wide - 2) == 0))) return 17;
        volatile int signed_x = (int)(x & 0x7fffffffu) - 0x40000000;
        if (b6_signed_select(signed_x) != (signed_x < 0 ? 0u : signed_x == 0)) return 18;
    }
    return 0;
}

#endif /* BATCH6_BENCH */
