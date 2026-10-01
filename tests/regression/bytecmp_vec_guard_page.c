/*
 * Byte-compare window phase: the q-side window must never cross a page
 * boundary into an unmapped page.
 *
 * The shape `while (p < end && *p == *q) { p++; q++; }` is vectorized by
 * putting a WIDTH-byte window phase in front of the scalar loop.  Only the
 * p stream is bounded by the loop's own `end`; `q` has no bound to reuse, so
 * the q window is issued only when the whole window stays inside the 4 KiB
 * page that contains q.  Every path into the phase implies `p < end`, so the
 * scalar loop itself reads q[0] and that page is mapped -- a window inside it
 * cannot fault, whatever the object's size.
 *
 * Without that rule the phase reads up to WIDTH-1 bytes past a byte the
 * scalar program reads, and that is a FAULT, not a slow path: with the page
 * test neutered (mutation test, 2026-10-01) this file's binary takes SIGSEGV
 * and exits 139.  With the test in place the answer is byte-identical to
 * GCC's, which never vectorizes this loop in the first place.
 *
 * Layout: one mapped page whose tail backs `q`, followed by a PROT_NONE
 * guard page.  Cases (every one of them is executable by the SCALAR loop
 * without touching a guard byte -- the test would otherwise fault for GCC
 * too):
 *
 *   edge-3     q's last valid byte is the page's last byte, mismatch at 3:
 *              the scalar loop reads q[0..3]; any window issued here crosses.
 *   edge-0     same placement, mismatch at 0 (the very first byte).
 *   tail-47    q has 48 readable bytes and the mismatch is at 47: the FIRST
 *              window is page-safe and the one after the 32-byte advance is
 *              not, so this pins the in-loop test, not only the entry test.
 *   short-17   the p bound is 20 bytes, below both window widths: the phase
 *              must fall through to the scalar tail.
 *   safe-*     page-safe controls (q well inside the page, > bound's worth of
 *              bytes readable): the fast path runs and must agree.
 *
 * A wrong exit pointer is a failure by itself; the printed checksum is
 * additionally compared against GCC's stdout by run_regression.py.
 */
#include <stdio.h>
#include <string.h>
#include <sys/mman.h>

#define PAGE 4096
#define P_LEN 256
#define Q_SAFE_OFF 512 /* offset in the mapped page for page-safe q */

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

static unsigned char pbuf[P_LEN];

static void fill(unsigned char *dst, int n, unsigned seed)
{
    for (int i = 0; i < n; i++)
        dst[i] = (unsigned char)((i * 7 + seed * 13 + 1) & 0xff);
}

static int failures;

/* p runs over [pbuf, pbuf+p_len); q is filled to match p, with byte
 * `mismatch` (when < p_len) flipped.  `q_readable`/`q_flip_ok` describe how
 * many bytes of q the scalar loop is allowed to read. */
static long run_case(const char *label, unsigned char *q, int q_readable,
                     int p_len, int mismatch)
{
    fill(pbuf, P_LEN, 0);
    fill(q, q_readable, 0);
    if (mismatch < p_len) {
        if (mismatch >= q_readable) {
            printf("TEST-BUG %s: flip at %d outside %d readable q bytes\n",
                   label, mismatch, q_readable);
            failures++;
            return 0;
        }
        q[mismatch] ^= 0xff;
    }
    const unsigned char *r = bc_scan(pbuf, pbuf + p_len, q);
    long got = (long)(r - pbuf);
    long want = (mismatch < p_len) ? mismatch : p_len;
    if (got != want) {
        printf("MISMATCH %s: got %ld want %ld\n", label, got, want);
        failures++;
    }
    return got;
}

int main(void)
{
    unsigned char *base = mmap(NULL, 2 * PAGE, PROT_READ | PROT_WRITE,
                               MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if (base == MAP_FAILED) {
        /* No mmap: the guard cannot be exercised here.  Still comparable:
         * GCC prints the same line for the same reason. */
        printf("setup mmap-failed\n");
        return 0;
    }
    if (mprotect(base + PAGE, PAGE, PROT_NONE) != 0) {
        printf("setup mprotect-failed\n");
        return 0;
    }

    long sum = 0;
    /* Guard-page placements: q ends at the last byte of the mapped page. */
    sum += run_case("edge-3", base + PAGE - 4, 4, P_LEN, 3);
    sum += run_case("edge-0", base + PAGE - 4, 4, P_LEN, 0);
    sum += run_case("tail-47", base + PAGE - 48, 48, P_LEN, 47);
    /* Page-safe controls: the fast path runs and must produce the same
     * answers, including a run longer than any single window. */
    sum += run_case("safe-early", base + Q_SAFE_OFF, P_LEN, P_LEN, 3);
    sum += run_case("safe-mid", base + Q_SAFE_OFF, P_LEN, P_LEN, 47);
    sum += run_case("safe-late", base + Q_SAFE_OFF, P_LEN, P_LEN, P_LEN - 1);
    sum += run_case("safe-full", base + Q_SAFE_OFF, P_LEN, P_LEN, P_LEN);
    sum += run_case("short-17", base + Q_SAFE_OFF, P_LEN, 20, 17);

    /* The signed-char loop (I8 loads) through the same page-safe control. */
    {
        unsigned char *q = base + Q_SAFE_OFF;
        fill(pbuf, P_LEN, 3);
        fill(q, P_LEN, 3);
        const char *rs = bc_scan_signed((const char *)pbuf,
                                        (const char *)(pbuf + P_LEN),
                                        (const char *)q);
        long got = (long)(rs - (const char *)pbuf);
        if (got != P_LEN) {
            printf("MISMATCH signed-full: got %ld want %d\n", got, P_LEN);
            failures++;
        }
        sum += got;
    }

    printf("checksum=%ld failures=%d\n", sum, failures);
    return failures == 0 ? 0 : 1;
}
