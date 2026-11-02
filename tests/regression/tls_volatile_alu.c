/* Defined unsigned arithmetic; changing each slot prevents a constant-load
 * hoist from accidentally satisfying the runtime oracle. The paired assembly
 * contract separately checks that the intended fusion actually happens. */
#include <stdint.h>
#include <stdio.h>

static __thread volatile uint32_t words[3];
static __thread volatile uint64_t wide[3];

__attribute__((noinline)) static uint32_t mix32(uint32_t n) {
    uint32_t result = UINT32_C(0x81234567);
    for (uint32_t i = 0; i < n; ++i) {
        words[1] = i * UINT32_C(0x9e3779b9) + 1;
        result += words[1];
        result -= words[1];
        result ^= words[1];
        result |= words[1];
        result &= words[1];
        result *= words[1];
        result += words[1] < result;
        result ^= words[1] == result;
    }
    return result;
}

__attribute__((noinline)) static uint64_t mix64(uint32_t n) {
    uint64_t result = UINT64_C(0x8123456789abcdef);
    for (uint32_t i = 0; i < n; ++i) {
        wide[1] = (uint64_t)i * UINT64_C(0x9e3779b97f4a7c15) + 1;
        result += wide[1];
        result -= wide[1];
        result ^= wide[1];
        result |= wide[1];
        result &= wide[1];
        result *= wide[1];
        result += wide[1] < result;
        result ^= wide[1] == result;
    }
    return result;
}

int main(void) {
    static const uint32_t counts[] = {0, 1, 2, 31, 100003};
    for (unsigned i = 0; i < sizeof(counts) / sizeof(counts[0]); ++i) {
        uint32_t a = mix32(counts[i]);
        uint64_t b = mix64(counts[i]);
        printf("%08x %016llx\n", (unsigned)a, (unsigned long long)b);
    }
    return 0;
}
