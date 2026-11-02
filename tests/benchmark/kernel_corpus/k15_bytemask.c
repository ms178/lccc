/* Count zero bytes: compare + conditional increment. */
#include <stdint.h>

uint32_t cntz(const uint8_t *p, uint32_t n) {
    uint32_t c = 0;
    for (uint32_t i = 0; i < n; i++) if (p[i] == 0) c++;
    return c;
}
