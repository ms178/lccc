/*
 * Shape fixture for the AVX2 strided-2 32-bit sum-reduction vectorizer
 * (src/passes/vectorize_strided_i32_reduce.rs).
 *
 * Two functions, because the transform has two dereferenceability proofs and
 * only one of them is exercised by a global array:
 *
 *   sum_global   reads a locally binding GlobalAddr, so `global_min_bytes`
 *                proves every 32-byte vector lies inside the object and the
 *                whole vector prefix is used;
 *   sum_dynamic  reads through a run-time pointer, so the transform may only
 *                vectorize prefixes that stay inside the bytes the scalar loop
 *                itself dereferences, and the untouched scalar loop finishes
 *                the remainder.
 *
 * Both compute a modular 32-bit sum of `dword(buf + i) ^ (uint32_t)i` with a
 * byte induction variable stepping by 8, which is the pattern the pass
 * documents. The lengths below include a zero trip count, a length that is not
 * a multiple of the 32-byte vector step, and the exact boundary cases the
 * `bound <= type_max - 8` admission test guards.
 *
 * The companion gate compares the printed line against GCC for every length and
 * asserts both bodies still contain packed 256-bit dword arithmetic: a scalar
 * fallback would otherwise satisfy the arithmetic comparison vacuously.
 */
#include <stdint.h>
#include <stdio.h>

#define ELEMENTS 4096U /* dwords; 2048 scalar iterations of 8 bytes */

static uint32_t g_buf[ELEMENTS];

__attribute__((noinline))
static uint32_t sum_global(uint64_t len)
{
    uint32_t acc = 0;
    for (uint64_t i = 0; (i + 8) <= len; i += 8)
        acc += *(const uint32_t *)((const unsigned char *)g_buf + i) ^ (uint32_t)i;
    return acc;
}

__attribute__((noinline))
static uint32_t sum_dynamic(const unsigned char *buf, uint64_t len)
{
    uint32_t acc = 0;
    for (uint64_t i = 0; (i + 8) <= len; i += 8)
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

    /* Byte lengths: 0 (zero trip), below one vector step, exactly one vector
     * step, an in-between length that leaves a scalar remainder, and the whole
     * buffer. */
    static const uint64_t lengths[] = {0U, 4U, 8U, 24U, 32U, 40U, 4096U,
                                       (uint64_t)sizeof(g_buf)};
    const unsigned char *heap = (const unsigned char *)g_buf;

    for (uint32_t k = 0; k < (uint32_t)(sizeof(lengths) / sizeof(lengths[0])); k++) {
        uint64_t len = lengths[k];
        if (len > sizeof(g_buf))
            continue;
        printf("%08x %08x\n", sum_global(len), sum_dynamic(heap, len));
    }
    return 0;
}
