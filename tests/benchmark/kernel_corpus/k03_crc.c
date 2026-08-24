/* Table-driven CRC-32: SIB-addressed table load in the loop. */
#include <stdint.h>

extern const uint32_t table[256];

uint32_t crc32k(const uint8_t *p, uint32_t n) {
    uint32_t c = 0xFFFFFFFFu;
    for (uint32_t i = 0; i < n; i++) c = table[(c ^ p[i]) & 0xFF] ^ (c >> 8);
    return c ^ 0xFFFFFFFFu;
}
