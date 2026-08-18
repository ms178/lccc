/* Call-free integer/pointer parameters should use caller-saved homes without
 * callee-save prologue overhead.  Six arguments exercise homes that overlap
 * incoming ABI registers, so the prologue must solve a parallel-copy problem. */
#include <stdint.h>
#include <stdio.h>

volatile uint64_t inputs[6] = {1, 2, 3, 4, 5, 6};

__attribute__((noinline))
uint64_t mix6(uint64_t a, uint64_t b, uint64_t c,
              uint64_t d, uint64_t e, uint64_t f) {
    return a + 3*b + 5*c + 7*d + 11*e + 13*f;
}

__attribute__((noinline))
uint64_t pointer_mix(const uint64_t *a, const uint64_t *b,
                     uint64_t i, uint64_t j) {
    return a[i] * 17 + b[j] * 19;
}

int main(void) {
    uint64_t a = inputs[0], b = inputs[1], c = inputs[2];
    uint64_t d = inputs[3], e = inputs[4], f = inputs[5];
    uint64_t m = mix6(a, b, c, d, e, f);
    uint64_t p = pointer_mix((const uint64_t *)inputs,
                             (const uint64_t *)inputs, 1, 4);
    printf("%llu %llu\n", (unsigned long long)m, (unsigned long long)p);
    return m == 183 && p == 129 ? 0 : 1;
}
