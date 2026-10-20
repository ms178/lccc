/* glibc memmem/strstr family shape: Boyer-Moore-Horspool substring search.
 * Skip-table driven scan with a short verification loop on candidate hits --
 * the algorithmic core glibc's two-way implementation also reduces to on
 * typical inputs.  Exercises skip-table loads, data-dependent loop exits,
 * and the inner byte-compare loop; needles are placed so both the skip path
 * and the verify path stay hot.  Returns match offsets folded into a
 * checksum so a search that finds less cannot look faster. */
#include <stddef.h>
#include <stdint.h>
#include <string.h>
#include "bench.h"

#define N 32768
static unsigned char hay[N];
static const unsigned char needle[] = "the quick brown fox";
#define NLEN (sizeof(needle) - 1)

void bench_setup(void) {
    static const char text[] =
        "pack my box with five dozen liquor jugs. how vexingly quick daft "
        "zebras jump! the five boxing wizards jump quickly. ";
    size_t tl = sizeof(text) - 1;
    for (size_t i = 0; i < N; i++) hay[i] = (unsigned char)text[i % tl];
    /* Plant true matches at irregular intervals. */
    for (size_t at = 1000; at + NLEN < N; at += 2999) memcpy(hay + at, needle, NLEN);
}

static uint64_t horspool_all(const unsigned char *h, size_t hn,
                             const unsigned char *nd, size_t nn) {
    size_t skip[256];
    for (int i = 0; i < 256; i++) skip[i] = nn;
    for (size_t i = 0; i + 1 < nn; i++) skip[nd[i]] = nn - 1 - i;
    uint64_t acc = 0, hits = 0;
    size_t i = 0;
    while (i + nn <= hn) {
        size_t j = nn;
        while (j > 0 && h[i + j - 1] == nd[j - 1]) j--;
        if (j == 0) { acc += i; hits++; i += 1; }
        else i += skip[h[i + nn - 1]];
    }
    return acc * 131u + hits;
}

int bench_verify(void) {
    /* Independent naive O(n*m) search over the same haystack: Horspool's
     * skip table must find exactly the same offsets.  Two different
     * algorithms agreeing is the oracle; a skip table with an off-by-one
     * still produces a self-consistent bench checksum. */
    uint64_t acc = 0, hits = 0;
    for (size_t i = 0; i + NLEN <= N; i++) {
        if (memcmp(hay + i, needle, NLEN) == 0) { acc += i; hits++; }
    }
    if (hits == 0) return 1;  /* setup plants matches: zero means stale */
    return horspool_all(hay, N, needle, NLEN) != acc * 131u + hits;
}

unsigned long long bench_run(void) { return horspool_all(hay, N, needle, NLEN); }
