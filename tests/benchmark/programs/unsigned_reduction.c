/* Unsigned-counter / unsigned-lane reduction kernels.
 *
 * The systems-C idiom written with `size_t` or `unsigned` induction
 * variables and uint32_t/uint64_t accumulators — checksum, hash-combine
 * and statistics code. Before unsigned reduction vectorization the packed
 * reduction path required a signed strict exit and signed lanes, so every
 * kernel here stayed scalar even though packed paddd/paddq and the low-half
 * multiplies are pure two's-complement bit operations, identical for the
 * signed and unsigned same-width types.
 *
 * Every datum keeps the high bit set so a sign-extending lane would change
 * the answer immediately; the unsigned arithmetic is defined to wrap, which
 * also makes the final checksum independent of operation association.
 * Bounds arrive through the ABI as runtime values, matching call sites in
 * real parsers rather than compile-time trip counts.
 */
#include <stdint.h>
#include <stddef.h>
#include <stdio.h>

#ifndef REDUCTION_PASSES
#define REDUCTION_PASSES 256
#endif

enum { N = 1u << 16, PASSES = REDUCTION_PASSES };

static uint32_t a32[N], b32[N];
static uint64_t a64[N], b64[N];

uint32_t sum_u32(const uint32_t *a, size_t n) {
    uint32_t s = 0;
    for (size_t i = 0; i < n; i++) s += a[i];
    return s;
}

uint32_t sum_u32_u32iv(const uint32_t *a, unsigned n) {
    uint32_t s = 0;
    for (unsigned i = 0; i < n; i++) s += a[i];
    return s;
}

uint64_t sum_u64(const uint64_t *a, size_t n) {
    uint64_t s = 0;
    for (size_t i = 0; i < n; i++) s += a[i];
    return s;
}

uint32_t dot_u32(const uint32_t *a, const uint32_t *b, size_t n) {
    uint32_t s = 0;
    for (size_t i = 0; i < n; i++) s += a[i] * b[i];
    return s;
}

uint64_t dot_u64(const uint64_t *a, const uint64_t *b, size_t n) {
    uint64_t s = 0;
    for (size_t i = 0; i < n; i++) s += a[i] * b[i];
    return s;
}

int main(void) {
    uint32_t seed = 0x9e3779b9u;
    for (size_t i = 0; i < N; i++) {
        seed = seed * 1664525u + 1013904223u;
        a32[i] = seed | 0x80000000u;
        seed = seed * 1664525u + 1013904223u;
        b32[i] = seed ^ 0x55555555u;
        a64[i] = (uint64_t)a32[i] << 32 | seed;
        b64[i] = (uint64_t)b32[i] << 32 | (seed ^ 0xa5a5a5a5u);
    }

    uint64_t checksum = 0;
    for (int p = 0; p < PASSES; p++) {
        checksum += sum_u32(a32, N);
        checksum += sum_u32_u32iv(b32, N);
        checksum += sum_u64(a64, N);
        checksum += dot_u32(a32, b32, N);
        checksum += dot_u64(a64, b64, N);
    }

    printf("%016llx\n", (unsigned long long)checksum);
    return 0;
}
