/* Byte-sum reduction: load + widen + accumulate. */
#include <stdint.h>

unsigned int sum8(const uint8_t *p, uint32_t n) {
    unsigned int s = 0;
    for (uint32_t i = 0; i < n; i++) s += p[i];
    return s;
}
