/*
 * Shape fixture for the AVX2 strided-2 32-bit sum-reduction vectorizer
 * (src/passes/vectorize_strided_i32_reduce.rs).
 *
 * Pointer forms require exact-width reads of scalar elements; their
 * intervening bytes may be independently shared or device-mapped. Sized
 * globals additionally exercise proven wide loads, including a non-16-byte
 * offset. Both full groups and a scalar tail are exercised. A protected page begins
 * immediately after the last scalar dword of the full-group pointer case.
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
#include <sys/mman.h>
#include <unistd.h>

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
static uint32_t sum_global_unaligned(void)
{
    const unsigned char *buf = (const unsigned char *)g_buf + 4;
    uint32_t acc = 0;
    for (uint64_t i = 0; i < BYTES_FULL; i += 8)
        acc += *(const uint32_t *)(buf + i) ^ (uint32_t)i;
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

__attribute__((noinline))
static uint32_t sum_dynamic_full(const unsigned char *buf)
{
    uint32_t acc = 0;
    for (uint64_t i = 0; i < BYTES_FULL; i += 8)
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

    long page = sysconf(_SC_PAGESIZE);
    if (page <= 0) return 1;
    size_t span = ((BYTES_FULL + (size_t)page - 1) / (size_t)page) * (size_t)page;
    unsigned char *map = mmap(0, span + (size_t)page, PROT_READ | PROT_WRITE,
                              MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if (map == MAP_FAILED) return 2;
    if (mprotect(map + span, (size_t)page, PROT_NONE)) return 3;
    unsigned char *exact = map + span - (BYTES_FULL - 4);
    const unsigned char *source = (const unsigned char *)g_buf;
    for (unsigned i = 0; i < BYTES_FULL - 4; ++i) exact[i] = source[i];
    uint32_t full = sum_global_full();
    if (sum_dynamic_full(exact) != full) return 4;
    if (munmap(map, span + (size_t)page)) return 5;
    printf("%08x %08x %08x\n", full, sum_dynamic_tail((const unsigned char *)g_buf),
           sum_global_unaligned());
    return 0;
}
