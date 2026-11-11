/* The byte-search idiom must NOT fire when the load index is a narrowing cast.
 *
 * `for (i = 0; i < n; i++) if (p[(unsigned short)i] == c) return p + i;`
 * reads address `p + (i mod 65536)` while the value it returns is `p + i`.
 * `memchr` scans `p[0..n)` linearly, so rewriting this loop changes which
 * bytes are examined -- and the rewrite discards the load address entirely,
 * keeping only `p + i`.  Before the narrowing-cast check in `operand_is_iv`
 * this program printed 65539 here and -1 under GCC and Clang.
 *
 * The buffer is sized so the wrapped index matters: the only occurrence of
 * the needle sits at 65536 + 3, i.e. address 3 for the loop (which holds a
 * different byte) and address 65539 for a linear scan.
 */
#include <stdio.h>
#include <stdlib.h>

#define N 70000

static unsigned char *buf;

__attribute__((noinline)) static const unsigned char *scan(unsigned char *p, unsigned char c,
                                                           size_t n) {
    for (size_t i = 0; i < n; i++)
        if (p[(unsigned short)i] == c) return p + i;
    return 0;
}

/* A same-width reinterpret (`(long)i`-style index casts) stays exact and must
 * keep matching: this one is the plain search with an explicit index cast. */
__attribute__((noinline)) static const unsigned char *scan_ok(unsigned char *p, unsigned char c,
                                                              size_t n) {
    for (size_t i = 0; i < n; i++)
        if (p[(unsigned long)i] == c) return p + i;
    return 0;
}

int main(void) {
    int bad = 0;
    buf = calloc(N, 1);
    for (int i = 0; i < N; i++) buf[i] = 1;

    buf[65536 + 3] = 7;
    const unsigned char *r = scan(buf, 7, N);
    printf("trunc %ld\n", r ? (long) (r - buf) : -1L);
    if (r != 0) bad++;

    buf[65536 + 3] = 1;
    buf[1234] = 9;
    const unsigned char *k = scan_ok(buf, 9, N);
    printf("plain %ld\n", k ? (long) (k - buf) : -1L);
    if (k != buf + 1234) bad++;

    /* Boundary of the wrapped range: a match at index 0 is found at i = 0. */
    buf[1234] = 1;
    buf[0] = 5;
    const unsigned char *z = scan(buf, 5, N);
    printf("zero %ld\n", z ? (long) (z - buf) : -1L);
    if (z != buf) bad++;

    free(buf);
    return bad;
}
