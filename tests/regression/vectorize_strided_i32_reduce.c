/*
 * Shape fixture for the AVX2 strided-2 32-bit sum-reduction vectorizer
 * (src/passes/vectorize_strided_i32_reduce.rs).
 *
 * Two functions, because the transform has two dereferenceability proofs and
 * only one of them is exercised by a global array:
 *
 *   sum_global_full   reads a locally binding GlobalAddr, so `global_min_bytes`
 *                     proves every 32-byte vector lies inside the object and the
 *                     whole vector prefix is used (trip count a multiple of 4);
 *   sum_dynamic_tail  reads through a run-time pointer, so the transform may
 *                     only vectorize prefixes that stay inside the bytes the
 *                     scalar loop itself dereferences, and the untouched scalar
 *                     loop finishes the remainder (trip count 2049, i.e. one
 *                     iteration left over).
 *
 * Both accumulate the modular 32-bit sum of `dword(buf + i) ^ (uint32_t)i` with
 * a byte induction variable stepping by 8. The loop is written in the form the
 * pass admits: the exit test compares the induction variable itself against a
 * COMPILE-TIME constant (`analyze` requires `Operand::Const` on the right-hand
 * side and matches the compared value to the IV phi, so neither a run-time bound
 * nor an `i + 8 <= bound` spelling reaches it).
 *
 * The companion gate compares the printed result against GCC and asserts both
 * bodies still contain packed 256-bit dword arithmetic: a scalar fallback would
 * otherwise satisfy the arithmetic comparison vacuously.
 */
#include <stdint.h>
#include <stdio.h>

#define BYTES_FULL 16384U /* 2048 iterations of 8 bytes: trip count % 4 == 0 */
#define BYTES_TAIL 16392U /* 2049 iterations: one scalar iteration left over */
#define ELEMENTS 4200U    /* dwords; covers BYTES_TAIL / 4 with slack */

static uint32_t g_buf[ELEMENTS];

__attribute__((noinline))
static uint32_t sum_global_full(void)
{
    uint32_t acc = 0;
    for (uint64_t i = 0; i < BYTES_FULL; i += 8)
        acc += *(const uint32_t *)((const unsigned char *)g_buf + i) ^ (uint32_t)i;
    return acc;
}

__attribute__((noinline))
static uint32_t sum_dynamic_tail(const unsigned char *buf)
{
    uint32_t acc = 0;
    for (uint64_t i = 0; i < BYTES_TAIL; i += 8)
        acc += *(const uint32_t *)(buf + i) ^ (uint32_t)i;
    return acc;
}

int main(void)
{
    uint32_t s = 0x1234567U;
    for (uint32_t i = 0; i < ELEMENTS; i++) {
        s = s * 1664525U + 1013904223U;
        g_buf[i] = s ^ (s >> 13);
    }

    printf("%08x %08x\n", sum_global_full(),
           sum_dynamic_tail((const unsigned char *)g_buf));
    return 0;
}
