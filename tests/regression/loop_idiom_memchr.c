/* Differential fixture for the `loop_idiom` byte-search rewrite.
 *
 * `scan` is the canonical shape:
 *
 *     for (size_t i = 0; i < n; i++) if (p[i] == c) return p + i;
 *     return 0;
 *
 * At -O2 the pass replaces it with `memchr(p, (int)(unsigned char)c, n)`.
 * This fixture is the semantic half of the gate: EVERY needle value (so the
 * signedness of the comparison is exercised on both sides of 0x80), EVERY
 * length from 0 to 257, and every position including the not-found case.
 * The checksum is produced by GCC 16.2 on the same source; a difference here
 * is a miscompile, whether the rewrite fired or not.  The shape half (that
 * the call is emitted, and that the switches and near misses keep the loop)
 * lives in `tests/corpus/test_loop_idiom_memchr.py`.
 */
#include <stddef.h>
#include <stdio.h>

#define N 257
#define EXPECTED 3523653933815388291ULL

static unsigned char buf[N];

static const unsigned char *scan(const unsigned char *p, unsigned char c, size_t n) {
    for (size_t i = 0; i < n; i++)
        if (p[i] == c) return p + i;
    return 0;
}

int main(void) {
    for (int i = 0; i < N; i++) buf[i] = (unsigned char)((i * 37 + 11) & 0xff);
    unsigned long long h = 1469598103934665603ULL;
    for (int c = 0; c < 256; c++) {
        for (size_t n = 0; n <= N; n++) {
            const unsigned char *r = scan(buf, (unsigned char)c, n);
            unsigned long long d = r ? (unsigned long long)(r - buf) : (unsigned long long)N + 1;
            h = (h ^ d) * 1099511628211ULL;
        }
    }
    printf("%llu\n", h);
    return h == EXPECTED ? 0 : 1;
}
