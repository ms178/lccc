#include <stdint.h>

/* The separately compiled pointer kernel cannot assume what lives in gaps. */
__attribute__((noinline))
uint32_t strided_watch_kernel(const unsigned char *p) {
    uint32_t acc = 0;
    for (uint64_t i = 0; i < 1024; i += 8)
        acc += *(const uint32_t *)(p + i) ^ (uint32_t)i;
    return acc;
}
