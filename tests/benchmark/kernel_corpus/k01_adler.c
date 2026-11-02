/* Adler-32 DO8 inner loop: byte loads + two dependent accumulators. */
#include <stdint.h>

uint32_t adler8(uint32_t adler, const uint8_t *p, uint32_t n) {
    uint32_t s1 = adler & 0xffff;
    uint32_t s2 = adler >> 16;
    while (n >= 8) {
        s1 += *p++; s2 += s1;
        s1 += *p++; s2 += s1;
        s1 += *p++; s2 += s1;
        s1 += *p++; s2 += s1;
        s1 += *p++; s2 += s1;
        s1 += *p++; s2 += s1;
        s1 += *p++; s2 += s1;
        s1 += *p++; s2 += s1;
        n -= 8;
    }
    while (n--) { s1 += *p++; s2 += s1; }
    return (s2 << 16) | s1;
}
