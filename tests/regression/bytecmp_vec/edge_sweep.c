/* Red-team probe for the byte-compare window phase.
 *
 * Upstream's guard driver parks q at ONE safe offset (512).  The guard under
 * test -- `q + (WIDTH-1) <= (q | 4095)` -- has all of its meaning at the page
 * END, so the offsets that matter are the last WIDTH-1 bytes of a page: there
 * the scalar program's reads are still legal, the window crosses into the
 * PROT_NONE page, and the phase must DECLINE and hand back to the scalar loop.
 * This sweeps every one of those offsets, and does the same for the p side,
 * where the bound the room test enforces is `end`.
 *
 * The distinction that matters: the scalar program reads q[0..n-1] only.  The
 * window phase wants to read q[0..WIDTH-1].  So the mismatch index m is kept
 * strictly inside the page while the window is not -- that is precisely the
 * case where a missing or inverted guard turns into a wild read.
 */
#define _GNU_SOURCE
#include <stdio.h>
#include <string.h>
#include <sys/mman.h>
#include <unistd.h>

typedef unsigned char u8;

const unsigned char *bytecmp_unsigned(const unsigned char *p,
                                      const unsigned char *end,
                                      const unsigned char *q);

static int bad = 0;

int main(void)
{
    long ps = sysconf(_SC_PAGESIZE);
    size_t page = (size_t)ps;
    const size_t W = 64;   /* >= AVX2 (32) and SSE2 (16) window spans */

    /* ---- q page-boundary sweep ---------------------------------------- */
    u8 *base = mmap(NULL, 2 * page, PROT_READ | PROT_WRITE,
                    MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if (base == MAP_FAILED) { printf("SKIP mmap\n"); return 0; }
    if (mprotect(base + page, page, PROT_NONE) != 0) {
        printf("SKIP mprotect\n");
        return 0;
    }

    /* p is a private, page-safe buffer. */
    u8 *pbuf = mmap(NULL, page, PROT_READ | PROT_WRITE,
                    MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if (pbuf == MAP_FAILED) { printf("SKIP mmap2\n"); return 0; }
    for (size_t i = 0; i < 64; i++) pbuf[i] = (u8)(i + 1);

    /* The DANGEROUS configuration, which a naive sweep misses:
         - p is LONG (so the phase's room test `p+31 < end` passes and the
           phase actually runs), and
         - q's mismatch is EARLY (the scalar loop reads only q[0..m-1] and
           exits), and
         - q sits close enough to the page end that q[0..WIDTH-1] crosses
           into the PROT_NONE page.
       Tying p's length to m makes the room test fail and the phase never
       runs, which is why a first attempt at this sweep passed even with the
       page guard deleted.  Here p is 64 bytes regardless of m. */
    const size_t PLEN = 64;
    for (size_t i = 0; i < PLEN; i++) pbuf[i] = (u8)(i + 1);

    for (size_t off = page - W; off < page; off++) {
        u8 *q = base + off;
        size_t readable = page - off;   /* bytes of q the program may touch */
        /* m is the index of the first differing byte: the scalar loop reads
           exactly q[0..m-1].  Restrict m to the readable range so the program
           is legal, but allow m well below WIDTH so the window over-reads. */
        for (size_t m = 1; m <= readable && m <= 40; m++) {
            for (size_t i = 0; i < m; i++) q[i] = pbuf[i];
            q[m - 1] = (u8)(pbuf[m - 1] ^ 0xFF);

            const u8 *r = bytecmp_unsigned(pbuf, pbuf + PLEN, q);
            long got = (long)(r - pbuf);
            if (got != (long)(m - 1)) {
                printf("BAD q_off=%zu m=%zu got=%ld want=%zu\n",
                       off, m, got, m - 1);
                bad++;
            }
        }
    }
    munmap(pbuf, page);
    munmap(base, 2 * page);

    /* ---- p bound at the page edge ------------------------------------- */
    u8 *pbase = mmap(NULL, 2 * page, PROT_READ | PROT_WRITE,
                     MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if (pbase != MAP_FAILED) {
        if (mprotect(pbase + page, page, PROT_NONE) == 0) {
            u8 *qbuf = mmap(NULL, page, PROT_READ | PROT_WRITE,
                            MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
            if (qbuf != MAP_FAILED) {
                /* `end` sits at the very end of the mapped page, so the room
                   test `p + (WIDTH-1) < end` is the only thing standing
                   between the phase and a read past the guard page. */
                for (size_t len = 0; len <= 40; len++) {
                    for (size_t m = 0; m <= len; m++) {
                        size_t n = len;
                        u8 *p = pbase + (page - n);
                        memset(p, 'a', n);
                        memcpy(qbuf, p, n);
                        if (m < n) qbuf[m] = 'b';
                        const u8 *r = bytecmp_unsigned(p, p + n, qbuf);
                        long got = (long)(r - p);
                        long want = (m < n) ? (long)m : (long)n;
                        if (got != want) {
                            printf("BAD p len=%zu m=%zu got=%ld want=%ld\n",
                                   n, m, got, want);
                            bad++;
                        }
                    }
                }
                munmap(qbuf, page);
            }
        }
        munmap(pbase, 2 * page);
    }

    if (bad) { printf("FAIL %d mismatches\n", bad); return 1; }
    printf("PASS bytecmp page/bounds sweep\n");
    return 0;
}
