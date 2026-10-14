/*
 * IVSR-AFFINE-ADD-WRAP-1: an affine offset must respect the type of the Add
 * that forms it, not only the type of the later multiply or pointer offset.
 *
 * `safe_affine` is a positive control: (i + 2) is formed in U32 before the
 * address calculation, but its constant trip range proves that it cannot wrap.
 * `wrapping_affine` executes exactly once with i == UINT32_MAX - 1, so the
 * source U32 addition wraps to zero and reads data[0].  Widening the recurrence
 * before that Add would instead form an address near 2^32 elements away.
 */
#include <stdint.h>
#include <stdio.h>

static uint32_t data[66];

__attribute__((noinline)) static uint32_t safe_affine(const uint32_t *p) {
    uint32_t sum = 0;
    for (uint32_t i = 0; i < 64; ++i)
        sum += p[i + 2U];
    return sum;
}

__attribute__((noinline)) static uint32_t wrapping_affine(const uint32_t *p) {
    uint32_t sum = 0;
    for (uint32_t i = UINT32_MAX - 1U; i < UINT32_MAX; ++i)
        sum += p[i + 2U];
    return sum;
}

int main(void) {
    for (uint32_t i = 0; i < 66; ++i)
        data[i] = i * 17U + 9U;

    /* Fixed answers are deliberately independent of a second loop compiled
     * by the candidate compiler: sum(data[2..66)) is 37024; the wrapping
     * one-trip access is data[0] == 9. */
    if (safe_affine(data) != 37024U) {
        fputs("safe U32 affine address mismatch\n", stderr);
        return 1;
    }
    if (wrapping_affine(data) != 9U) {
        fputs("wrapping U32 affine address mismatch\n", stderr);
        return 2;
    }
    puts("ivsr_affine_add_wrap: OK");
    return 0;
}
