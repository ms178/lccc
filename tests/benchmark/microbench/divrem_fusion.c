/* Synthetic before/after divider microbenchmark, not an end-to-end compiler
 * benchmark. Build with the adjacent .s file, e.g. gcc -O2 file.c file.s -o b.
 * Reports nanoseconds/call and checks that both variants have the same digest.
 * Alternating order reduces (but does not eliminate) frequency/order bias. */
#define _POSIX_C_SOURCE 200809L
#include <stdint.h>
#include <stdio.h>
#include <time.h>

uint64_t fused(uint64_t, uint64_t);
uint64_t unfused(uint64_t, uint64_t);

static double run(uint64_t (*f)(uint64_t, uint64_t), uint64_t *out) {
    struct timespec start, end;
    uint64_t sum = 0, x = UINT64_C(0x123456789abcdef0);
    clock_gettime(CLOCK_MONOTONIC, &start);
    for (unsigned i = 0; i < 3000000; i++) {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        sum += f(x, (x >> 32) | 1);
    }
    clock_gettime(CLOCK_MONOTONIC, &end);
    *out = sum;
    return (end.tv_sec - start.tv_sec) * 1e9 + (end.tv_nsec - start.tv_nsec);
}

int main(void) {
    for (int i = 0; i < 7; i++) {
        uint64_t a_digest, b_digest;
        double a, b;
        if (i & 1) {
            b = run(fused, &b_digest);
            a = run(unfused, &a_digest);
        } else {
            a = run(unfused, &a_digest);
            b = run(fused, &b_digest);
        }
        if (a_digest != b_digest) return 1;
        printf("%.3f %.3f ratio %.3f digest %llu\n", a / 3e6, b / 3e6, b / a,
               (unsigned long long)a_digest);
    }
    return 0;
}
