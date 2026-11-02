// Structural regression for -Os transitive nested-loop inline costing.
// Small helpers expand inside loop_kernel.  The pre-expansion body fits the
// nested-loop cap, but its post-inline body does not and must remain outlined
// when called from the loop in outer_loop.

static inline unsigned step1(unsigned x) { return (x ^ 0x9e3779b9u) + (x << 3); }
static inline unsigned step2(unsigned x) { return (x + 0x7f4a7c15u) ^ (x >> 5); }
static inline unsigned step3(unsigned x) { return (x ^ (x << 7)) + 0x6a09e667u; }
static inline unsigned step4(unsigned x) { return (x + (x >> 3)) ^ 0xbb67ae85u; }
static inline unsigned step5(unsigned x) { return (x ^ 0x3c6ef372u) + (x << 5); }
static inline unsigned step6(unsigned x) { return (x + 0xa54ff53au) ^ (x >> 7); }
static inline unsigned step7(unsigned x) { return (x ^ (x << 11)) + 0x510e527fu; }
static inline unsigned step8(unsigned x) { return (x + (x >> 13)) ^ 0x1f83d9abu; }

static unsigned loop_kernel(unsigned x, unsigned count)
{
    while (count--) {
        x = step1(x);
        x = step2(x);
        x = step3(x);
        x = step4(x);
        x = step5(x);
        x = step6(x);
        x = step7(x);
        x = step8(x);
    }
    return x;
}

unsigned outer_loop(unsigned rounds)
{
    unsigned acc = 0x12345678u;
    while (rounds--) {
        acc ^= loop_kernel(acc + rounds, (rounds & 7u) + 1u);
        acc += loop_kernel(acc ^ rounds, (rounds & 3u) + 2u);
    }
    return acc;
}

#if defined(TEST_MAIN)
#include <stdio.h>
#include <stdlib.h>
int main(int argc, char **argv)
{
    unsigned n = argc > 1 ? (unsigned)strtoul(argv[1], 0, 0) : 1000u;
    printf("%u\n", outer_loop(n));
    return 0;
}
#elif !defined(STRUCTURAL_ONLY)
int main(void)
{
    return outer_loop(3) == 1124822167u ? 0 : 1;
}
#endif
