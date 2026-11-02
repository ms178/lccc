/*
 * range-check folding ((x>=lo && x<=hi) -> (unsigned)(x-lo) <= hi-lo).
 *
 * Exercises every shape the range_check pass folds (inclusive &&, exclusive
 * ||, collapsed ==, reversed empty ranges, signed/unsigned, all widths, and
 * the cast-chain matching), plus the untouched single-comparison controls.
 * Differential vs GCC via the harness (--compare-gcc): output is a 64-bit
 * checksum over every byte value and INT/LLONG edge values.
 */
#include <stdio.h>
#include <limits.h>

static int az(unsigned char c) { return c >= 'a' && c <= 'z'; }
static int azAZ(unsigned char c) { return (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z'); }
static int digit(unsigned char c) { return c >= '0' && c <= '9'; }
static int az_digit(unsigned char c) { return (c >= 'a' && c <= 'z') || (c >= '0' && c <= '9'); }
static int wide_u(unsigned int c) { return c >= 0x61u && c <= 0x7au; }
static int sig(int c) { return c >= -5 && c <= 5; }
static int sig_or(int c) { return c < -5 || c > 5; }
static int ll(long long c) { return c >= -100LL && c <= 100LL; }
static int eq3(int c) { return c >= 3 && c <= 3; }
static int reversed(int c) { return c >= 5 && c <= -5; } /* always 0 */
static int utf8_lead(unsigned char c) { return c >= 0xc2U && c <= 0xdfU; }
static int u16range(unsigned short c) { return c >= 50000u && c <= 60000u; }
static int negated(unsigned char c) { return !(c >= 'a' && c <= 'z'); }
static int as_data(unsigned char c) { return (c >= 'a' && c <= 'z') ? 11 : 22; } /* value use, not branch */

static unsigned long long mix(unsigned long long h, unsigned long long v) {
    return (h ^ v) * 0x9e3779b97f4a7c15ULL + 0xdeadbeefULL;
}

int main(void) {
    unsigned long long h = 1469598103934665603ULL;
    int c;
    for (c = 0; c <= 255; c++) {
        unsigned char u = (unsigned char)c;
        h = mix(h, (unsigned)az(u));
        h = mix(h, (unsigned)azAZ(u));
        h = mix(h, (unsigned)digit(u));
        h = mix(h, (unsigned)az_digit(u));
        h = mix(h, (unsigned)utf8_lead(u));
        h = mix(h, (unsigned)negated(u));
        h = mix(h, (unsigned)as_data(u));
        h = mix(h, (unsigned)u16range((unsigned short)(c * 257u)));
    }
    {
        static const int vals[] = { INT_MIN, INT_MIN + 1, -101, -100, -99, -6, -5, -4,
            0, 2, 3, 4, 5, 6, 99, 100, 101, INT_MAX - 1, INT_MAX };
        unsigned i;
        for (i = 0; i < sizeof(vals) / sizeof(vals[0]); i++) {
            int v = vals[i];
            h = mix(h, (unsigned)sig(v));
            h = mix(h, (unsigned)sig_or(v));
            h = mix(h, (unsigned)eq3(v));
            h = mix(h, (unsigned)reversed(v));
            h = mix(h, (unsigned)wide_u((unsigned)v));
        }
    }
    {
        static const long long lv[] = { LLONG_MIN, LLONG_MIN + 1, -101, -100, -99, 0,
            99, 100, 101, LLONG_MAX - 1, LLONG_MAX };
        unsigned i;
        for (i = 0; i < sizeof(lv) / sizeof(lv[0]); i++)
            h = mix(h, (unsigned)ll(lv[i]));
    }
    printf("%llu\n", (unsigned long long)h);
    return 0;
}
