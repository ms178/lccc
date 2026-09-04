/* CPU tuning model — inline expansion of fixed-size memset (follow-up item
 * "memset lowering", docs/CPU_MODEL_AUDIT.md §4).
 *
 * Every constant-size memset used to be `call memset@PLT`.  The expansion is
 * now chosen per tuning row (X86Tune::memset_strategy): overlapping scalar
 * stores (< 16 B), straight-line vector stores (≤ 8 × vector width), a
 * counted vector loop, `rep stosb` at/above glibc's rep_stosb_threshold on
 * ERMS rows, and the libc call above the L3-derived bound.
 *
 * The test pins the *observable* contract for every path:
 *   - exactly [0, N) is written (32-byte canaries on both sides survive the
 *     overlapping stores and the negative-displacement remainder store);
 *   - the fill is the low byte of the int argument (C11 7.24.6.1), for
 *     constants 0 / 0xFF / 0x5A and for runtime values (incl. 0x1234, -1);
 *   - memset returns its first argument;
 *   - argument-register order does not matter (fill byte homed in %rdi);
 *   - alloca, global and pointer-arithmetic destinations all work;
 *   - the pointer stays valid after the expansion (the loop path advances
 *     %rdi internally).
 * Compared against GCC by the suite runner. */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define SIZES(X) \
    X(0) X(1) X(2) X(3) X(4) X(5) X(6) X(7) X(8) X(9) X(10) X(11) X(12) X(13) X(14) X(15) \
    X(16) X(17) X(23) X(24) X(31) X(32) X(33) X(40) X(47) X(48) X(63) X(64) X(65) X(79) X(80) \
    X(95) X(96) X(100) X(127) X(128) X(129) X(130) X(144) X(159) X(160) X(191) X(192) X(200) \
    X(255) X(256) X(257) X(263) X(264) X(271) X(272) X(287) X(288) X(300) X(319) X(320) \
    X(511) X(512) X(513) X(1000) X(1023) X(1024) X(1025) X(2047) X(2048) X(2049) X(2111) \
    X(2112) X(2113) X(4095) X(4096) X(4097) X(8191) X(8192) X(8193) X(12345) X(20000)

#define DEF(N) \
    static __attribute__((noinline)) void *fz##N(void *p) { return memset(p, 0, N); } \
    static __attribute__((noinline)) void *ff##N(void *p) { return memset(p, 0xFF, N); } \
    static __attribute__((noinline)) void *fa##N(void *p) { return memset(p, 0x5A, N); } \
    static __attribute__((noinline)) void *fr##N(void *p, int c) { return memset(p, c, N); } \
    static __attribute__((noinline)) void *fs##N(int c, void *p) { return memset(p, c, N); } \
    static __attribute__((noinline)) void *fb##N(void *p) { return __builtin_memset(p, 0x80, N); }
SIZES(DEF)

struct probe {
    unsigned n;
    void *(*z)(void *);
    void *(*f)(void *);
    void *(*a)(void *);
    void *(*r)(void *, int);
    void *(*s)(int, void *);
    void *(*b)(void *);
};
#define ROW(N) { N, fz##N, ff##N, fa##N, fr##N, fs##N, fb##N },
static const struct probe probes[] = { SIZES(ROW) };

enum { PAD = 32, MAXN = 20000 };
static unsigned char arena[PAD + MAXN + PAD];
static unsigned failures;

static void prime(void) {
    for (unsigned i = 0; i < sizeof arena; i++) arena[i] = (unsigned char)(0xC3 ^ (i * 7));
}

static void check(const char *what, unsigned n, unsigned char fill, void *ret, void *want) {
    if (ret != want) { printf("FAIL %s n=%u: returned %p want %p\n", what, n, ret, want); failures++; return; }
    unsigned start = (unsigned)((unsigned char *)want - arena);
    for (unsigned i = 0; i < sizeof arena; i++) {
        unsigned char exp = (i >= start && i < start + n) ? fill : (unsigned char)(0xC3 ^ (i * 7));
        if (arena[i] != exp) {
            printf("FAIL %s n=%u: byte %d = %02x want %02x\n", what, n, (int)i - (int)start, arena[i], exp);
            failures++;
            return;
        }
    }
}

/* Destination kinds beyond a pointer argument. */
struct s40 { unsigned char b[40]; };
static struct s40 g40;
static __attribute__((noinline)) unsigned sum_alloca(int c) {
    unsigned char local[72];
    unsigned s = 0;
    memset(local, c, sizeof local);
    for (unsigned i = 0; i < sizeof local; i++) s += local[i];
    memset(local + 8, 0, 56);
    for (unsigned i = 0; i < sizeof local; i++) s += local[i] * (i + 1);
    return s;
}
static __attribute__((noinline)) unsigned sum_global(void) {
    memset(&g40, 0x11, sizeof g40);
    memset(g40.b + 3, 0x22, 20);
    unsigned s = 0;
    for (unsigned i = 0; i < sizeof g40; i++) s += g40.b[i] * (i + 1);
    return s;
}
/* Pointer live after the expansion; two fills back to back. */
static __attribute__((noinline)) unsigned two_fills(unsigned char *p) {
    memset(p, 1, 300);
    memset(p + 300, 2, 300);
    unsigned s = 0;
    for (unsigned i = 0; i < 600; i++) s += p[i] * (i + 1);
    return s;
}

int main(void) {
    volatile int rc = 0x1234;      /* low byte 0x34 */
    volatile int rneg = -1;        /* low byte 0xFF */
    void *p = arena + PAD;
    for (unsigned k = 0; k < sizeof probes / sizeof probes[0]; k++) {
        const struct probe *pr = &probes[k];
        prime(); check("zero", pr->n, 0x00, pr->z(p), p);
        prime(); check("ones", pr->n, 0xFF, pr->f(p), p);
        prime(); check("0x5a", pr->n, 0x5A, pr->a(p), p);
        prime(); check("rt34", pr->n, 0x34, pr->r(p, rc), p);
        prime(); check("rtff", pr->n, 0xFF, pr->r(p, rneg), p);
        prime(); check("swap", pr->n, 0x34, pr->s(rc, p), p);
        prime(); check("bltn", pr->n, 0x80, pr->b(p), p);
        /* Unaligned destination exercises every remainder shape again. */
        prime(); check("un+1", pr->n, 0x5A, pr->a(arena + PAD + 1), arena + PAD + 1);
        prime(); check("un+7", pr->n, 0x00, pr->z(arena + PAD + 7), arena + PAD + 7);
    }
    printf("alloca=%u global=%u two=%u\n", sum_alloca(0x2F), sum_global(), two_fills(arena));
    if (failures) { printf("FAILURES %u\n", failures); return 1; }
    puts("OK");
    return 0;
}
