/* Schoolbook bignum multiply with a __int128 accumulator (DA-02 oracle).
 *
 * This is the kernel shape that exposed lccc's widest measured codegen gap:
 * a 64x64->128 product plus two carry additions per inner step. A compiler
 * that legalises the wide value into a register pair emits one `mulq` and
 * an add/adc chain; a compiler that keeps __int128 in stack slots emits a
 * full 128x128 product (three multiplies) and reloads the accumulator on
 * every iteration. The bench harness compares the checksum across arms, so
 * a wrong-but-fast arm is reported as a correctness failure, and
 * bench_verify cross-checks one product against a 64-bit-only reference.
 */
#include <stddef.h>
#include <stdint.h>
#include "bench.h"

typedef unsigned __int128 u128;

#define NLIMBS 32
static uint64_t a[NLIMBS], b[NLIMBS], out[2 * NLIMBS];

void bench_setup(void) {
    for (int i = 0; i < NLIMBS; i++) {
        a[i] = 0x9e3779b97f4a7c15ULL * (uint64_t)(i + 1) + 0x1234567ULL;
        b[i] = 0xc2b2ae3d27d4eb4fULL * (uint64_t)(NLIMBS - i) + 0x7654321ULL;
        out[i] = out[i + NLIMBS] = 0;
    }
}

static uint64_t bn_mul(const uint64_t *x, const uint64_t *y, uint64_t *o, int n) {
    uint64_t acc = 0;
    for (int i = 0; i < n; i++) {
        u128 carry = 0;
        for (int j = 0; j < n; j++) {
            u128 t = (u128)x[i] * y[j] + o[i + j] + carry;
            o[i + j] = (uint64_t)t;
            carry = t >> 64;
        }
        o[i + n] = (uint64_t)carry;
        acc += o[i + n];
    }
    return acc;
}

int bench_verify(void) {
    /* 0xffffffffffffffff^2 = 0xfffffffffffffffe0000000000000001, computed
     * through the same kernel with n == 1. */
    uint64_t x = ~0ULL, y = ~0ULL, o[2] = {0, 0};
    (void)bn_mul(&x, &y, o, 1);
    return o[0] == 1 && o[1] == 0xfffffffffffffffeULL ? 0 : 1;
}

unsigned long long bench_run(void) {
    for (int i = 0; i < 2 * NLIMBS; i++) out[i] = 0;
    return bn_mul(a, b, out, NLIMBS) ^ out[NLIMBS] ^ out[2 * NLIMBS - 1];
}
