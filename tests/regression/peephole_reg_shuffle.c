/* Execution coverage for the x86 register-shuffling peepholes.
 *
 * Each function below is a shape one of the passes rewrites; the expected
 * values are computed with plain C semantics, so a wrong rewrite (aliased
 * destination, wrong width, stale flags, resurrected dead register) shows up
 * as a value mismatch rather than as a diff in the assembly.
 *
 *   count_zero_bytes   setcc/movzbl/test/cmov  -> cmov on the comparison
 *   masked_table_sum   mov + and $255          -> movzbl
 *   scaled_index_sum   mov + shl $2            -> scaled lea
 *   bumped_copy        mov + add $imm          -> lea
 *   byte_relay         load + copy             -> producer retargeting
 *   wrap32             the width traps: a 32-bit add must wrap, a 64-bit one
 *                      must not; a movl copy must not be widened
 *   branch_after_bool  the boolean's flags feed a BRANCH after the cmov, so
 *                      the idiom must survive
 */
#include <stdint.h>

static int fails;
#define CHK(what, got, want)                                                                       \
    do {                                                                                           \
        unsigned long long g = (unsigned long long)(got), w = (unsigned long long)(want);          \
        if (g != w) {                                                                              \
            fails++;                                                                               \
            __builtin_printf("FAIL %s: got %llu want %llu\n", what, g, w);                         \
        }                                                                                          \
    } while (0)

__attribute__((noinline)) static uint32_t count_zero_bytes(const uint8_t *p, uint32_t n) {
    uint32_t c = 0;
    for (uint32_t i = 0; i < n; i++)
        if (p[i] == 0) c++;
    return c;
}

__attribute__((noinline)) static uint32_t masked_table_sum(const uint32_t *t, const uint32_t *k,
                                                           uint32_t n) {
    uint32_t s = 0;
    for (uint32_t i = 0; i < n; i++) s += t[k[i] & 0xff];
    return s;
}

__attribute__((noinline)) static int scaled_index_sum(const int *a, const int *idx, int n) {
    int s = 0;
    for (int i = 0; i < n; i++) s += a[idx[i]];
    return s;
}

__attribute__((noinline)) static void bumped_copy(int *dst, const int *src, int n) {
    for (int i = 0; i < n; i++) dst[i + 1] = src[i] + 1;
}

__attribute__((noinline)) static uint64_t byte_relay(const uint8_t *p, uint32_t n) {
    uint64_t h = 0;
    for (uint32_t i = 0; i < n; i++) {
        uint64_t b = p[i];
        h = (h << 1) ^ b;
    }
    return h;
}

/* 32-bit arithmetic must wrap and zero-extend; 64-bit must not. */
__attribute__((noinline)) static uint64_t wrap32(uint32_t x, uint64_t y) {
    uint32_t a = x;
    a += 1; /* wraps at 2^32 */
    uint64_t b = y;
    b += 1; /* does not */
    return ((uint64_t)a << 32) ^ b;
}

/* The boolean feeds both a select and a branch: the select may not steal the
 * comparison out from under the branch. */
__attribute__((noinline)) static int branch_after_bool(int a, int b, int t, int f) {
    int cond = (a == b);
    int v = cond ? t : f;
    if (cond) v += 100;
    return v;
}

int main(void) {
    static const uint8_t bytes[16] = {0, 1, 0, 0, 5, 0, 7, 0, 0, 9, 0, 11, 12, 0, 0, 15};
    CHK("count_zero_bytes", count_zero_bytes(bytes, 16), 9u);
    CHK("count_zero_bytes/0", count_zero_bytes(bytes, 0), 0u);

    static const uint32_t tab[256] = {[0] = 7, [1] = 11, [255] = 13, [64] = 17};
    static const uint32_t keys[5] = {0u, 0x100u, 0x1ffu, 0xff40u, 1u};
    /* keys & 0xff -> 0, 0, 255, 64, 1  => 7 + 7 + 13 + 17 + 11 */
    CHK("masked_table_sum", masked_table_sum(tab, keys, 5), 55u);

    static const int arr[8] = {10, 20, 30, 40, 50, 60, 70, 80};
    static const int idx[5] = {7, 0, 3, 3, 5};
    CHK("scaled_index_sum", scaled_index_sum(arr, idx, 5), 80 + 10 + 40 + 40 + 60);

    int dst[6] = {0, 0, 0, 0, 0, 0};
    const int src[5] = {1, 2, 3, 4, 5};
    bumped_copy(dst, src, 5);
    CHK("bumped_copy", dst[0] + dst[1] * 10 + dst[5], 0 + 20 + 6);

    {
        uint64_t h = 0;
        for (int i = 0; i < 16; i++) h = (h << 1) ^ bytes[i];
        CHK("byte_relay/ref", byte_relay(bytes, 16), h);
    }

    CHK("wrap32", wrap32(0xffffffffu, 0xffffffffull), ((uint64_t)0 << 32) ^ 0x100000000ull);
    CHK("wrap32/2", wrap32(1u, 1ull), ((uint64_t)2 << 32) ^ 2ull);

    CHK("branch_after_bool/eq", branch_after_bool(3, 3, 7, 9), 107);
    CHK("branch_after_bool/ne", branch_after_bool(3, 4, 7, 9), 9);

    return fails;
}
