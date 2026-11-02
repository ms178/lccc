/* PF-17 / Guard D: `while (--i)` must not rotate into an infinite loop.
 *
 * gzip huft_build does:
 *   i = g;
 *   x[1] = j = 0;
 *   p = c + 1; xp = x + 2;
 *   while (--i) { *xp++ = (j += *p++); }
 *
 * The IV update `i_next = i - 1` lives in the header (it IS the guard).
 * Cloning that subtract onto the latch and rewriting the phi use to the
 * latch incoming turns `i_next' = i - 1` into `i_next' = (g-1) - 1`, a
 * loop-invariant. The backedge then never exits for g > 2 and the pointer
 * walk SIGSEGVs. This test is the same shape with a bounded buffer. */
#include <stdio.h>
#include <stdint.h>

__attribute__((noinline))
uint32_t prefix_from_counts(const uint32_t *c, unsigned g) {
    uint32_t x[16];
    unsigned i = g;
    uint32_t j = 0;
    const uint32_t *p = c + 1;
    uint32_t *xp = x + 2;
    x[1] = 0;
    while (--i) {
        *xp++ = (j += *p++);
    }
    /* Fold the prefix table so a miscompile cannot hide behind "didn't crash". */
    uint32_t h = x[1];
    for (unsigned k = 2; k <= g; k++) {
        h = h * 16777619u ^ x[k];
    }
    h ^= j * 0x9e3779b1u;
    return h;
}

int main(void) {
    /* Bit-length counts similar to huft_build_crash's c[] after the first scan
     * of {4,0,0,7,4,4,4,2,3,3,4,4,4,0,0,0,5,7,6} — g ends up 7. */
    uint32_t c[16] = {0, 0, 1, 2, 8, 1, 1, 6, 0, 0, 0, 0, 0, 0, 0, 0};
    uint32_t h = prefix_from_counts(c, 7);
    printf("%u\n", h);
    return 0;
}
