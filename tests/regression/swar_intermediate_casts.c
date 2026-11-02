/* a9987d0 red-team audit: masks do not justify peeling intermediate casts.
 * Before the fix, low16(UINT_MAX) returned 32 instead of 16 at -O2.
 * Keep functions externally visible and noinline to prevent test folding.
 * Defined unsigned arithmetic only. No system headers / LP64 assumptions. */
extern int printf(const char *, ...);

#define SWAR(name, cast1, cast2, cast3) \
__attribute__((noinline)) unsigned name(unsigned x) { \
    x = cast1 (x - ((x >> 1) & 0x55555555u)); \
    x = cast2 ((x & 0x33333333u) + ((x >> 2) & 0x33333333u)); \
    x = cast3 (x + (x >> 4)) & 0x0f0f0f0fu; \
    return (x * 0x01010101u) >> 24; \
}
SWAR(swar_full, , , )
SWAR(swar_low16, , , (unsigned short))
SWAR(swar_low8, , , (unsigned char))
SWAR(swar_stage1, (unsigned short), , )
SWAR(swar_stage2, , (unsigned short), )

__attribute__((noinline)) unsigned count_bits(unsigned x, unsigned width) {
    unsigned n = 0;
    for (unsigned i = 0; i < width; i++) n += (x >> i) & 1u;
    return n;
}

int main(void) {
    unsigned state = 0x12345678u;
    for (unsigned i = 0; i < 4096; i++) {
        state = state * 1664525u + 1013904223u;
        unsigned x = i == 0 ? ~0u : i == 1 ? 0u : i < 34 ? 1u << (i-2) : state;
        unsigned full = count_bits(x, 32), low16 = count_bits(x, 16), low8 = count_bits(x, 8);
        if (swar_full(x) != full || swar_low16(x) != low16 || swar_low8(x) != low8 ||
            swar_stage1(x) != low16 || swar_stage2(x) != low16) {
            printf("FAIL %08x: %u %u %u %u %u (expected %u %u %u)\n", x,
                   swar_full(x), swar_low16(x), swar_low8(x), swar_stage1(x),
                   swar_stage2(x), full, low16, low8);
            return 1;
        }
    }
    printf("PASS: 4096 SWAR inputs, five shapes\n");
    return 0;
}
