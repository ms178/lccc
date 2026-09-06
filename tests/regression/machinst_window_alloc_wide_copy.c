/* MachInst window-allocator regression, WIDE-COPY-OF-SMALL-SLOT class.
 *
 * The shape that dominated the residual replay census (99 of 106 fallback
 * windows): a `main` that computes a batch of 32-bit results and stages
 * them into call/argument registers through Copies. The isel uses the S64
 * relay whenever a destination register must carry defined upper bits, so
 * every staging Copy reads an I32 value at 64-bit width. When those values
 * spill to 4-byte small slots the memory-operand substitution is refused
 * (an 8-byte read of a 4-byte slot pulls in the neighbour's garbage — the
 * resolver's width guard), and before the window allocator's narrow-slot
 * promotion each such window replayed through the text path
 * (MI-FALLBACK). The promotion classifies those vregs for window scratch
 * registers with typed S32 reloads (movl zero-extends: the full register
 * is defined), the same discipline the text path applies.
 *
 * Execution is bit-exact against a reference computed in the same binary;
 * the companion shell check verifies the window no longer replays.
 * GCC oracle. */
#include <stdio.h>

volatile unsigned seed_v = 7;

static unsigned mix(unsigned x) {
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    return x;
}

int main(void) {
    /* Seed through a volatile: the values stay runtime data. */
    unsigned s = seed_v;
    unsigned r0 = mix(s), r1 = mix(r0), r2 = mix(r1), r3 = mix(r2);
    unsigned r4 = mix(r3), r5 = mix(r4), r6 = mix(r5), r7 = mix(r6);
    unsigned r8 = mix(r7), r9 = mix(r8), r10 = mix(r9), r11 = mix(r10);
    unsigned r12 = mix(r11), r13 = mix(r12), r14 = mix(r13), r15 = mix(r14);

    /* One wide call consuming all sixteen 32-bit results: the argument
     * staging is the wide-Copy-of-small-slot shape (the printf promotion
     * puts the ints beyond the register bank into small spill slots). */
    printf("%u %u %u %u %u %u %u %u %u %u %u %u %u %u %u %u\n",
           r0, r1, r2, r3, r4, r5, r6, r7,
           r8, r9, r10, r11, r12, r13, r14, r15);

    unsigned e0 = mix(7), e1 = mix(e0), e2 = mix(e1), e3 = mix(e2);
    unsigned e4 = mix(e3), e5 = mix(e4), e6 = mix(e5), e7 = mix(e6);
    unsigned e8 = mix(e7), e9 = mix(e8), e10 = mix(e9), e11 = mix(e10);
    unsigned e12 = mix(e11), e13 = mix(e12), e14 = mix(e13), e15 = mix(e14);
    int ok = r0 == e0 && r1 == e1 && r2 == e2 && r3 == e3;
    ok &= r4 == e4 && r5 == e5 && r6 == e6 && r7 == e7;
    ok &= r8 == e8 && r9 == e9 && r10 == e10 && r11 == e11;
    ok &= r12 == e12 && r13 == e13 && r14 == e14 && r15 == e15;
    return ok ? 0 : 1;
}
