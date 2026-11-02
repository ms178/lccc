/*
 * Byte-compare window phase: exhaustive bounds/mismatch-position coverage.
 *
 * `while (p < end && *p == *q) { p++; q++; }` is vectorized with a WIDTH-byte
 * window phase in front of the scalar loop (32B AVX2 / 16B SSE2), then the
 * loop's exact exit state is reconstructed from the byte mask.  This driver
 * walks the two dimensions that decide whether the reconstruction is right:
 *
 *   length    0..399   -- below, at, and around both window widths, and far
 *                         past them (multiple window iterations)
 *   mismatch  0..len   -- every position, including "no mismatch" (`len`),
 *                         so the entry window, every in-loop window, the
 *                         exact-mismatch exit and the scalar tail are all hit
 *
 * for both stream types the arm accepts: `const unsigned char *` (U8 loads)
 * and `const char *` (I8 loads).  Both are checked against a closed-form
 * expectation computed here, so a wrong exit pointer fails the test by
 * itself; the printed checksum additionally has to be byte-identical to
 * GCC's stdout (run_regression.py compares them).
 *
 * The buffers are deliberately page-safe with 64 bytes of slack after the
 * loop bound: this file is about answers, not about the guard-page contract
 * (tests/regression/bytecmp_vec_guard_page.c covers that one).
 */
#include <stdio.h>
#include <string.h>

#define MAXLEN 400
#define SLACK 64

static unsigned char pbuf[MAXLEN + SLACK];
static unsigned char qbuf[MAXLEN + SLACK];

__attribute__((noinline)) static const unsigned char *bc_scan(
    const unsigned char *p, const unsigned char *end, const unsigned char *q)
{
    while (p < end && *p == *q) { p++; q++; }
    return p;
}

__attribute__((noinline)) static const char *bc_scan_signed(
    const char *p, const char *end, const char *q)
{
    while (p < end && *p == *q) { p++; q++; }
    return p;
}

int main(void)
{
    long checksum = 0;
    int bad = 0;

    for (int len = 0; len < MAXLEN; len++) {
        for (int unit = 0; unit < 2; unit++) {
            for (int m = 0; m <= len; m++) {
                for (int i = 0; i < len; i++)
                    pbuf[i] = (unsigned char)((i * 31 + 7) & 0xff);
                memcpy(qbuf, pbuf, (size_t)len);
                /* q[0..m-1] matches, q[m] differs; m == len means full match. */
                if (m < len)
                    qbuf[m] = (unsigned char)(pbuf[m] ^ 0x9b);

                long got;
                if (unit == 0) {
                    const unsigned char *r = bc_scan(pbuf, pbuf + len, qbuf);
                    got = (long)(r - pbuf);
                } else {
                    const char *r = bc_scan_signed((const char *)pbuf,
                                                   (const char *)(pbuf + len),
                                                   (const char *)qbuf);
                    got = (long)(r - (const char *)pbuf);
                }
                long want = (m < len) ? m : len;
                if (got != want) {
                    printf("BAD len=%d unit=%d m=%d got=%ld want=%ld\n",
                           len, unit, m, got, want);
                    if (++bad > 20)
                        return 1;
                }
                checksum += got;
            }
        }
    }

    printf("checksum=%ld bad=%d\n", checksum, bad);
    return bad != 0;
}
