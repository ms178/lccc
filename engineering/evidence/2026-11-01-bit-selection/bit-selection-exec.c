/* Differential execution for the single-bit idioms.
 *
 * Every value here is defined C: the bit index stays below the operand width,
 * so the shift `1u << n` is never undefined and GCC, Clang and LCCC must agree
 * bit for bit.  (The modulo-W behaviour *above* the width is LCCC's own
 * contract, checked by the matcher tests in `src/passes/bit_idioms_bit_select`
 * rather than by a C program, whose behaviour there is undefined.)
 *
 * The checksum folds in the low byte of every result so a single wrong bit
 * changes the exit status of the run. */
#include <stdio.h>
#include <stdint.h>

static unsigned h = 2166136261u;

static void mix(uint64_t v) {
    for (int i = 0; i < 8; i++) {
        h = (h ^ (unsigned)((v >> (8 * i)) & 0xFFu)) * 16777619u;
    }
}

static unsigned bset32(unsigned x, unsigned n)    { return x | (1u << n); }
static unsigned bclr32(unsigned x, unsigned n)    { return x & ~(1u << n); }
static int      btest32(unsigned x, unsigned n)   { return (x & (1u << n)) != 0; }
static unsigned bsetrf(unsigned x, unsigned n)    { unsigned y = x; y |= 1u << n; return y; }
static unsigned bclrrf(unsigned x, unsigned n)    { unsigned y = x; y &= ~(1u << n); return y; }

static uint64_t bset64(uint64_t x, uint64_t n)    { return x | (1ul << n); }
static uint64_t bclr64(uint64_t x, uint64_t n)    { return x & ~(1ul << n); }
static int      btest64(uint64_t x, uint64_t n)   { return (x & (1ul << n)) != 0; }

/* The same idioms at the sub-word widths the C frontend actually produces:
 * promotion means the shift happens at int width and the result is truncated,
 * which is why the fold refuses to create an 8/16-bit bit operation. */
static unsigned char bset8(unsigned char x, unsigned n) { return (unsigned char)(x | (1u << n)); }
static unsigned char bclr8(unsigned char x, unsigned n) { return (unsigned char)(x & ~(1u << n)); }
static int btest8(unsigned char x, unsigned n) { return (x & (1u << n)) != 0; }

/* Neighbours that must keep their meaning and (in the assembly checks) their
 * shape: a two-bit mask, a toggle, a mask test against a non-zero constant,
 * and the equality direction of the test. */
static unsigned keep_two(unsigned x, unsigned n)  { return x & (3u << n); }
static unsigned keep_xor(unsigned x, unsigned n)  { return x ^ (1u << n); }
static unsigned keep_shl(unsigned x, unsigned n)  { return (x << n) | x; }
static int      keep_eq4(unsigned x, unsigned n)  { return (x & (1u << n)) == 4; }
static int      btest32z(unsigned x, unsigned n)  { return (x & (1u << n)) == 0; }

int main(void) {
    static const unsigned patterns[] = {
        0x00000000u, 0xFFFFFFFFu, 0xAAAAAAAAu, 0x55555555u, 0x80000001u,
        0x00000002u, 0x00000001u, 0xdeadbeefu, 0x7fffffffu, 0x12345678u,
    };
    for (unsigned pi = 0; pi < sizeof patterns / sizeof patterns[0]; pi++) {
        unsigned x = patterns[pi];
        for (unsigned n = 0; n < 32; n++) {
            mix(bset32(x, n));
            mix(bclr32(x, n));
            mix(btest32(x, n));
            mix(btest32z(x, n));
            mix(bsetrf(x, n));
            mix(bclrrf(x, n));
            mix(keep_two(x, n));
            mix(keep_xor(x, n));
            mix(keep_eq4(x, n));
            mix((uint64_t)bset8((unsigned char)x, n & 7));
            mix((uint64_t)bclr8((unsigned char)x, n & 7));
            mix((uint64_t)btest8((unsigned char)x, n & 7));
        }
        for (unsigned n = 0; n < 32; n++) {
            mix(keep_shl(x, n));
        }
    }
    for (unsigned pi = 0; pi < sizeof patterns / sizeof patterns[0]; pi++) {
        uint64_t x = ((uint64_t)patterns[pi] << 32) | patterns[(pi + 3) % 10];
        for (unsigned n = 0; n < 64; n++) {
            mix(bset64(x, n));
            mix(bclr64(x, n));
            mix(btest64(x, n));
        }
    }
    printf("%08x\n", h);
    return 0;
}
