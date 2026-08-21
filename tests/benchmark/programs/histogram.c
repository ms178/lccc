// Deterministic 256-bin histogram kernel.
//
// Common database/parser/compression shape: indexed increment with a fixed-size
// table and a final reduction.  Self-contained and deterministic; not copied
// from any parent project.
#include <stdint.h>
#include <stddef.h>

enum { N = 1u << 18, BINS = 256 };
static unsigned char bytes[N];
static uint64_t bins[BINS];

int main(void) {
    for (unsigned i = 0; i < N; ++i) {
        unsigned x = (i * 2654435761u) >> 24;
        bytes[i] = (unsigned char)x;
    }
    for (unsigned i = 0; i < N; ++i)
        bins[bytes[i]]++;
    uint64_t total = 0, checksum = 0;
    for (unsigned b = 0; b < BINS; ++b) {
        total += bins[b];
        checksum = checksum * 131u + bins[b];
    }
    if (total != N)
        return 1;
    return checksum == 11058470557610399826ULL ? 0 : 2;
}
