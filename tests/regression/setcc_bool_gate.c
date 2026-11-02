/* setCC -> test -> cond-branch fusion (i686): the bool relay window is
 * dropped when the test is redundant, kept when the bool is live, and
 * REFUSED when a flags reader follows (a following jcc would read the
 * producer's flags after the rewrite). Every shape must match GCC. */
#include <stdio.h>
#include <stdint.h>

volatile uint32_t inputs[9] = {0u, 1u, 2u, 7u, 0x80000000u, 0xFFFFFFFFu, 3u, 12u, 0x7FFFFFFFu};

/* (v & mask) != 0 as a direct branch: window fully dead */
static uint32_t gated(uint32_t v) {
    if ((v & 0x80000000u) != 0) return v ^ 0x12345678u;
    return v + 1;
}

/* same test, bool STORED after the branch: window must survive */
static uint32_t gated_store(uint32_t v) {
    int big = (v & 0x80000000u) != 0;
    if (big) return v ^ 0x87654321u;
    return (uint32_t)big + v * 3;
}

/* inverted compare: == 0 as the taken side (je shape, sete producer) */
static uint32_t zero_or_not(uint32_t v) {
    int z = (v & 0xFFFFu) == 0;
    if (z) return 0xC0DEu;
    return v ^ 0xC0DEu;
}

/* a flags READER after the branch (carry from the mask-and is not used,
 * but the emitter's shape must still be exact): forced via inline asm
 * jcc on the SAME values is out of scope — instead use a nested test. */
static uint32_t chained(uint32_t v) {
    uint32_t r = (v & 1u) ? 10u : 20u;
    uint32_t q = (v & 2u) ? r + 1 : r - 1;
    return q;
}

int main(void) {
    for (int i = 0; i < 9; i++) printf("g%d=%08x\n", i, gated(inputs[i]));
    for (int i = 0; i < 9; i++) printf("s%d=%08x\n", i, gated_store(inputs[i]));
    for (int i = 0; i < 9; i++) printf("z%d=%08x\n", i, zero_or_not(inputs[i]));
    for (int i = 0; i < 9; i++) printf("c%d=%08x\n", i, chained(inputs[i]));
    return 0;
}
