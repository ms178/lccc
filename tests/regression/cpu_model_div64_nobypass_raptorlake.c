/* CPU tuning model — 64-bit division width bypass (`X86Tune::bypass_div64`).
 *
 * On Raptor Lake the div64 width bypass must be DISABLED: its tuning row
 * does not satisfy the bypass predicate.  This fixture drives div/rem
 * correctness without relying on a fast-path guard that must not exist.
 * The companion assembly gate checks the no-bypass code shape.
 *
 * This test drives division edge cases — upper halves zero / non-zero in
 * either operand, the 2^32 boundary, all sign combinations, div/rem pairs,
 * and a pseudo-random sweep — and checks each result against an independent
 * reference computed with __int128 (which never reaches the bypassed code).
 * INT64_MIN / -1 (and x / 0) trap by definition and are excluded; negation is
 * done in unsigned arithmetic so the test itself has no signed-overflow UB.
 */
#include <stdint.h>
#include <stdio.h>

#define NOINLINE __attribute__((noinline))

NOINLINE uint64_t udiv(uint64_t a, uint64_t b) { return a / b; }
NOINLINE uint64_t urem(uint64_t a, uint64_t b) { return a % b; }
NOINLINE int64_t sdiv(int64_t a, int64_t b) { return a / b; }
NOINLINE int64_t srem(int64_t a, int64_t b) { return a % b; }
/* div+rem of the same operands fuse into one divide (emit_divrem_pair_head). */
NOINLINE uint64_t upair(uint64_t a, uint64_t b, uint64_t *r) { *r = a % b; return a / b; }
NOINLINE int64_t spair(int64_t a, int64_t b, int64_t *r) { *r = a % b; return a / b; }

static uint64_t rng = 0x9E3779B97F4A7C15ull;
static uint64_t next(void) {
    rng ^= rng << 13; rng ^= rng >> 7; rng ^= rng << 17;
    return rng;
}

static int fails;
static void check_u(uint64_t a, uint64_t b) {
    if (b == 0) return;
    uint64_t q = (uint64_t)((unsigned __int128)a / b);
    uint64_t r = (uint64_t)((unsigned __int128)a % b);
    uint64_t pr, pq = upair(a, b, &pr);
    if (udiv(a, b) != q || urem(a, b) != r || pq != q || pr != r) {
        if (fails++ < 8) printf("FAIL u %llu %llu\n", (unsigned long long)a, (unsigned long long)b);
    }
}
static void check_s(int64_t a, int64_t b) {
    if (b == 0 || (a == INT64_MIN && b == -1)) return;
    int64_t q = (int64_t)((__int128)a / b);
    int64_t r = (int64_t)((__int128)a % b);
    int64_t pr, pq = spair(a, b, &pr);
    if (sdiv(a, b) != q || srem(a, b) != r || pq != q || pr != r) {
        if (fails++ < 8) printf("FAIL s %lld %lld\n", (long long)a, (long long)b);
    }
}

int main(void) {
    static const uint64_t edge[] = {
        0, 1, 2, 3, 7, 10, 255, 256, 65535, 65536,
        0x7FFFFFFFull, 0x80000000ull, 0xFFFFFFFFull, 0x100000000ull,
        0x100000001ull, 0x1FFFFFFFFull, 0x7FFFFFFFFFFFFFFFull,
        0x8000000000000000ull, 0x8000000000000001ull, 0xFFFFFFFF00000000ull,
        0xFFFFFFFFFFFFFFFEull, 0xFFFFFFFFFFFFFFFFull,
    };
    const unsigned n = sizeof edge / sizeof edge[0];
    for (unsigned i = 0; i < n; i++)
        for (unsigned j = 0; j < n; j++) {
            check_u(edge[i], edge[j]);
            check_s((int64_t)edge[i], (int64_t)edge[j]);
            check_s((int64_t)(0 - edge[i]), (int64_t)edge[j]);
            check_s((int64_t)edge[i], (int64_t)(0 - edge[j]));
        }
    /* Random sweep with operand widths drawn from 1..64 bits independently,
     * so all four (small|large) x (small|large) guard outcomes occur. */
    for (int k = 0; k < 200000; k++) {
        unsigned wa = 1 + next() % 64, wb = 1 + next() % 64;
        uint64_t a = next() >> (64 - wa), b = next() >> (64 - wb);
        check_u(a, b);
        check_s((int64_t)a, (int64_t)b);
        check_s((int64_t)(0 - a), (int64_t)b);
        check_s((int64_t)a, (int64_t)(0 - b));
    }
    if (fails) { printf("FAIL %d\n", fails); return 1; }
    printf("ok %llu %lld\n", (unsigned long long)udiv(0xFFFFFFFFull * 3, 7),
           (long long)srem((int64_t)(0 - 1000000007ull * 1000003ull), 97));
    return 0;
}
