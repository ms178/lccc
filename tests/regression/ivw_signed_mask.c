/* Explicit -fwrapv: positive masks prove a range; negative masks and overflowing
   following additions must not borrow that range. The unsigned reference uses
   defined modulo-32 arithmetic, separately from the signed kernels. */
#include <stdio.h>
#include <stdint.h>
#include <limits.h>
static volatile unsigned values[16];
#define KERNEL(name, mask, bias) \
__attribute__((noinline)) static unsigned name(void) { \
    unsigned sum = 0; \
    for (int i = 3; i < 64; ++i) \
        sum += values[((i & (mask)) + (bias)) & 15]; \
    return sum; \
}
KERNEL(positive, 7, 1)
KERNEL(zero, 0, 1)
KERNEL(top_room, INT_MAX - 1, 1)
KERNEL(top_wrap, INT_MAX, 1)
KERNEL(add_wrap, 7, INT_MAX)
KERNEL(negative, -1, 1)
KERNEL(negative_aligned, -8, 1)
KERNEL(sign_bit, INT_MIN, 1)
__attribute__((noinline)) static unsigned reference(uint32_t mask, uint32_t bias) {
    unsigned sum = 0;
    for (uint32_t i = 3; i < 64; ++i)
        sum += values[((i & mask) + bias) & 15u];
    return sum;
}
int main(void) {
    unsigned (*kernels[])(void) = {positive, zero, top_room, top_wrap, add_wrap,
                                  negative, negative_aligned, sign_bit};
    uint32_t masks[] = {7, 0, INT_MAX - 1u, INT_MAX, 7, UINT32_MAX,
                        UINT32_MAX - 7u, (uint32_t)INT_MIN};
    uint32_t biases[] = {1, 1, 1, 1, INT_MAX, 1, 1, 1};
    unsigned total = 0;
    for (unsigned i = 0; i < 16; ++i) values[i] = i * 19u + 3u;
    for (unsigned i = 0; i < 8; ++i) {
        unsigned actual = kernels[i]();
        if (actual != reference(masks[i], biases[i])) return 1 + i;
        total = total * 31u + actual;
    }
    printf("ivw-signed-mask %u\n", total);
    return total != 770224292u;
}
