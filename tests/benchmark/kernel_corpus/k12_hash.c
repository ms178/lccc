/* FNV-1a hash: byte load, xor, 64-bit multiply. */
#include <stdint.h>

uint64_t hsh(const uint8_t *p, uint32_t n) {
    uint64_t h = 14695981039346656037ULL;
    for (uint32_t i = 0; i < n; i++) { h ^= p[i]; h *= 1099511628211ULL; }
    return h;
}
