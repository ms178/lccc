/* Fixed input and checksum; argv[1] selects random, sparse, or dense words.
 * A noinline boundary keeps the population-count recurrence observable in
 * emitted assembly. scripts/measure_popcount_loops.py checks the checksum
 * independently and records timing distributions, not a single best run. */
#include <stdint.h>
#include <stdio.h>

#define WORDS 4096u
#define REPEATS 8192u
static uint64_t words[WORDS];

__attribute__((noinline))
static unsigned count(uint64_t x) {
    unsigned n = 0;
    while (x) {
        x &= x - 1;
        ++n;
    }
    return n;
}

int main(int argc, char **argv) {
    unsigned mode = argc > 1 ? (unsigned)(argv[1][0] - '0') : 0;
    if (mode > 2) return 2;
    uint64_t state = 1234567;
    for (unsigned i = 0; i < WORDS; ++i) {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        uint64_t bit = UINT64_C(1) << (i & 63);
        words[i] = mode == 0 ? state : (mode == 1 ? bit : ~bit);
    }
    uint64_t sum = 0;
    for (unsigned repeat = 0; repeat < REPEATS; ++repeat)
        for (unsigned i = 0; i < WORDS; ++i)
            sum += count(words[i]);
    printf("%llu\n", (unsigned long long)sum);
    return 0;
}
