/* In-place byte swap: load, bswap, store. */
#include <stdint.h>

void bswp32(uint32_t *p, uint32_t n) {
    for (uint32_t i = 0; i < n; i++) p[i] = __builtin_bswap32(p[i]);
}
