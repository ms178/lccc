/* CFG/phi-web stack-location regression. */
#include <stdint.h>
#include <stdio.h>

static uint64_t rot(uint64_t x, unsigned n) {
    n &= 63u;
    return n ? ((x << n) | (x >> ((64u - n) & 63u))) : x;
}

static uint64_t kernel(uint64_t seed) {
    uint64_t a = seed ^ UINT64_C(0x9e3779b97f4a7c15);
    uint64_t b = seed + UINT64_C(0xbf58476d1ce4e5b9);
    uint64_t c = UINT64_C(0x94d049bb133111eb);
    for (unsigned i = 0; i < 97; ++i) {
        switch ((unsigned)((a ^ b ^ c ^ i) & 3u)) {
        case 0: a = rot(a + b, i); b ^= c + i; break;
        case 1: b = rot(b + c, i + 7u); c ^= a + i; break;
        case 2: c = rot(c + a, i + 13u); a ^= b + i; break;
        default: a ^= rot(c, i); b += a ^ i; break;
        }
        if (a & 1u) {
            uint64_t old = b++;
            c ^= old + i;
        } else {
            uint64_t old = c--;
            a += old ^ i;
        }
    }
    return a ^ rot(b, 17) ^ rot(c, 39);
}

int main(void) {
    uint64_t got = kernel(UINT64_C(0x0123456789abcdef));
    const uint64_t expected = UINT64_C(0x549ed8b2506b8434);
    printf("%016llx\n", (unsigned long long)got);
    return got != expected;
}
