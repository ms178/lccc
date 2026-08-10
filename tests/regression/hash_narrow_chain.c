/* Regression: I64->U32 store-consumer narrowing (narrow.rs Phase 6).
 * The lowering widens C int arithmetic to I64 on x86-64; Phase 6 re-narrows
 * chains like `h = ((h << 5) ^ v) & mask` when every use is a truncating
 * store, so codegen emits 32-bit ops. This test verifies the result bits are
 * identical to a straight 64-bit computation (i.e. narrowing is bit-exact).
 */
#include <stdint.h>
#include <stdio.h>

static uint32_t g_ins_h;
static uint8_t g_buf[256];

static uint32_t hash_loop(void) {
    g_ins_h = 0;
    for (int i = 0; i < 256; i++) {
        uint32_t v = g_buf[i];
        g_ins_h = ((g_ins_h << 5) ^ v);
        g_ins_h &= 32767u;
    }
    return g_ins_h;
}

/* unsigned (zero-extend) path */
static uint32_t hash_unsigned(const uint8_t *buf, unsigned n) {
    uint32_t h = 0;
    for (unsigned i = 0; i < n; i++) {
        h = ((h << 5) ^ buf[i]) & 32767u;
    }
    return h;
}

/* signed (sign-extend) path: negative index arithmetic must stay correct */
static int32_t sum_signed(int32_t *v, int32_t n) {
    int32_t acc = 0;
    for (int32_t i = n - 1; i >= 0; i--) {
        acc = (acc + v[i]) & 0x7ffff;
    }
    return acc;
}

int main(void) {
    for (int i = 0; i < 256; i++) g_buf[i] = (uint8_t)(i * 7 + 3);

    uint32_t r1 = hash_loop();
    /* reference: same computation with explicit 64-bit math */
    uint64_t ref = 0;
    for (int i = 0; i < 256; i++) {
        uint64_t v = g_buf[i];
        ref = ((ref << 5) ^ v) & 32767u;
    }
    if (r1 != (uint32_t)ref) { printf("FAIL hash_loop: %u vs %llu\n", r1, (unsigned long long)ref); return 1; }

    uint8_t data[64];
    for (int i = 0; i < 64; i++) data[i] = (uint8_t)(255 - i);
    if (hash_unsigned(data, 64) != 4320u) { printf("FAIL hash_unsigned\n"); return 2; }

    int32_t vals[8] = { 1, -2, 3, -4, 5, -6, 7, -8 };
    if (sum_signed(vals, 8) != 524284) { printf("FAIL sum_signed\n"); return 3; }
    if (sum_signed(vals, 0) != 0) { printf("FAIL sum_signed(0)\n"); return 4; }

    printf("HASHNARROW-OK\n");
    return 0;
}
