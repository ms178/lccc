/* Regression: post-increment indexed addressing (`i++; a[i]`) folds into a
 * SIB memory operand anchored at the ADD's result, and alloca bases fold as
 * `disp(%rbp/%rsp,%idx,scale)`. Two historical defects are covered:
 *
 * 1. The PF-06 iv-update-copy gate refused the fold entirely when the RA
 *    coalesces the induction variable with the add's result — the scan
 *    loop then paid `movq %r13,%rbx; shlq $3,%rbx` per iteration
 *    (linux_find_bit). The retarget folds `a[++i]` as `0(base, i_new, 8)`.
 *
 * 2. The dead-offset producer walk required use_count == 1 per chain node
 *    and stopped at SHARED offsets (`a[i]; b[i]` both scale the same `i*8`),
 *    leaving the scaled-offset computation live as dead code.
 *
 * 3. OverAligned allocas (alignas(32)) must be rejected by the alloca-SIB
 *    gate — accepting them skipped the offset chain that the emitter's
 *    rematerialise fallback then read through a never-written register
 *    (vzeroupper_after_ymm compare loop SEGV). Differential vs GCC. */
#include <stdio.h>
#include <stdalign.h>

#define N 4096

static unsigned long words_a[N];
static unsigned long words_b[N];
alignas(32) static unsigned char over_dst[128];
alignas(32) static unsigned char over_src[128];

static unsigned long scan_andnot(const unsigned long *a,
                                 const unsigned long *b,
                                 unsigned long size, unsigned long start) {
    unsigned long mask, index, value, result = size;
    if (start >= size)
        return result;
    mask = ~0UL << (start % 64);
    index = start / 64;
    value = (a[index] & ~b[index]) & mask;
    while (!value) {
        if ((index + 1UL) * 64UL >= size)
            return result;
        index++;
        value = a[index] & ~b[index];
    }
    result = index * 64UL + __builtin_ctzl(value);
    return result < size ? result : size;
}

int main(void) {
    /* Sparse bitmap: one searchable bit every 64 words. */
    for (unsigned long i = 0; i < N; i++) {
        words_a[i] = 0UL;
        words_b[i] = ~0UL;
        if ((i & 63UL) == 5UL) {
            words_a[i] = 1UL << ((i * 13UL) & 63UL);
            words_b[i] = 0UL;
        }
    }
    unsigned long checksum = 0;
    unsigned long offset = 0;
    do {
        unsigned long bit = scan_andnot(words_a, words_b, N * 64UL, offset);
        if (bit < N * 64UL) {
            checksum ^= bit + 7;
            offset = bit + 1;
        } else {
            break;
        }
    } while (offset < N * 64UL);

    /* OverAligned copy: the alloca-SIB gate must refuse (runtime-aligned
     * address), the address math must stay correct. */
    for (int i = 0; i < 128; i++)
        over_src[i] = (unsigned char)(i * 3 + 1);
    for (int i = 0; i < 128; i += 16)
        for (int j = 0; j < 16; j++)
            over_dst[i + j] = over_src[i + j];
    unsigned long over_sum = 0;
    for (int i = 0; i < 128; i++)
        over_sum = over_sum * 31UL + over_dst[i];

    /* Shared-offset loops: `dst[i] = src[i]*3 + 7` over two allocas. */
    int la[100], lb[100];
    for (int i = 0; i < 100; i++)
        la[i] = i - 50;
    for (int i = 0; i < 100; i++)
        lb[i] = la[i] * 3 + 7;
    long lb_sum = 0;
    for (int i = 0; i < 100; i++)
        lb_sum += lb[i] * (long)(i + 1);

    printf("%lu %lu %ld\n", checksum, over_sum, lb_sum);
    return 0;
}
