/*
 * range-fold + compare-narrowing edge cases.
 *
 * Span 0 (x == c), full-range (span 255), lo == 0, hi == 255, signed byte
 * ranges, unsigned short ranges, and ranges that do NOT narrow (span too
 * wide). Exhaustive over byte values and edge values for wider types.
 */
#include <stdio.h>
#include <limits.h>

static int span0(unsigned char c) { return c >= 97 && c <= 97; }        /* == c */
static int fullbyte(unsigned char c) { return c >= 0 && c <= 255; }    /* always 1 */
static int lo0(unsigned char c) { return c >= 0 && c <= 10; }          /* c <= 10 */
static int hi255(unsigned char c) { return c >= 200 && c <= 255; }
static int span255(unsigned char c) { return c >= 0 && c <= 255; }
static int signbyte(signed char c) { return c >= -5 && c <= 5; }
static int signbyte2(signed char c) { return c >= -128 && c <= 127; }
static int ushortrange(unsigned short c) { return c >= 50000u && c <= 50025u; }
static int ushortwide(unsigned short c) { return c >= 0u && c <= 60000u; }
static int wide32(unsigned c) { return c >= 0u && c <= 0x7fffffffu; }
static int signbyte_or(signed char c) { return c < -5 || c > 5; }
static int u8or(unsigned char c) { return c < 10 || c > 200; }

static unsigned long long mix(unsigned long long h, unsigned long long v) {
    return (h ^ v) * 0x9e3779b97f4a7c15ULL + 0xdeadbeefULL;
}

int main(void) {
    unsigned long long h = 1469598103934665603ULL;
    int c;
    for (c = 0; c <= 255; c++) {
        unsigned char u = (unsigned char)c;
        signed char s = (signed char)(c - 128);
        h = mix(h, (unsigned)span0(u));
        h = mix(h, (unsigned)fullbyte(u));
        h = mix(h, (unsigned)lo0(u));
        h = mix(h, (unsigned)hi255(u));
        h = mix(h, (unsigned)span255(u));
        h = mix(h, (unsigned)signbyte(s));
        h = mix(h, (unsigned)signbyte2(s));
        h = mix(h, (unsigned)signbyte_or(s));
        h = mix(h, (unsigned)u8or(u));
    }
    unsigned short s;
    for (s = 0; s < 4096; s += 17) {
        h = mix(h, (unsigned)ushortrange(s * 16u));
        h = mix(h, (unsigned)ushortwide(s * 16u));
    }
    {
        static const unsigned xs[] = { 0, 1, 0x7fffffffu, 0x80000000u, 0xfffffffeu, 0xffffffffu };
        unsigned n;
        for (n = 0; n < sizeof(xs) / sizeof(xs[0]); n++)
            h = mix(h, (unsigned)wide32(xs[n]));
    }
    printf("%llu\n", (unsigned long long)h);
    return 0;
}
