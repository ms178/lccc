/* Regression: scalar loads through a just-computed pointer may dereference
   the accumulator directly; unsigned narrow loads must still target a valid
   32-bit destination and preserve their C value. */
#include <stdint.h>
#include <stdio.h>

static unsigned char bytes[257];

static uint64_t walk(const unsigned char *p, unsigned n) {
    uint64_t sum = 0;
    for (unsigned i = 0; i < n; ++i) {
        const unsigned char *q = p + i;
        unsigned char a = *q;
        unsigned char b = *(q + 1);
        sum += (uint64_t)a * 257u + b;
    }
    return sum;
}

int main(void) {
    for (unsigned i = 0; i < 257; ++i)
        bytes[i] = (unsigned char)((i * 37u + 11u) & 255u);
    uint64_t got = walk(bytes, 256);
    uint64_t expected = 0;
    for (unsigned i = 0; i < 256; ++i)
        expected += (uint64_t)bytes[i] * 257u + bytes[i + 1];
    printf("%llu %llu\n", (unsigned long long)got, (unsigned long long)expected);
    return got != expected;
}
