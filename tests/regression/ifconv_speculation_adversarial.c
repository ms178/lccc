/*
 * Adversarial probes for the if-conversion speculative-load coverage rule.
 *
 * The rule (arm_load_speculation_ok) permits hoisting an arm's load when its
 * canonical address key is dereferenced on every path pred->merge. Two things
 * the key/coverage pair must get right, both probed here with a page-guarded
 * allocation so an over-eager speculation FAULTS instead of silently reading
 * adjacent heap:
 *
 *   A. SIZE COVERAGE. The key names an ADDRESS, not an access extent. If the
 *      covering dereference is narrower than the speculated load, the extra
 *      bytes were never proven dereferenceable:
 *
 *          if (c) x = *(long *)p;   else  y = *(char *)p;
 *
 *      With `p` at the last byte of a mapping the char load is fine and the
 *      long load faults. A key-only coverage test says "covered".
 *
 *   B. KEY INJECTIVITY THROUGH CASTS. canonical_addr_key_impl walks through
 *      `Cast` unconditionally. A WIDENING cast is value-preserving, but a
 *      TRUNCATING one is not: `d[(int) big]` and `d[big]` are different
 *      addresses that must not share a key, or one arm's load/store is
 *      rewritten to the other's address.
 *
 *   C. SCALE INJECTIVITY. `d[i]` and `d[2*i]` differ only by the Shl constant;
 *      collapsing through Shl without recording it merged them.
 *
 * Every kernel is also run against a plain-C reference computed with volatile
 * inputs so a wrong-address rewrite shows up as a value mismatch even when it
 * does not fault.
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <unistd.h>

static long fails;

#define CHECK(cond, ...)                                                                 \
    do {                                                                                 \
        if (!(cond)) {                                                                   \
            printf(__VA_ARGS__);                                                         \
            ++fails;                                                                     \
        }                                                                                \
    } while (0)

/* ---- guarded buffer: `n` readable bytes followed by an unmapped page ---- */
static unsigned char *guarded(size_t n, size_t *out_pagesz) {
    size_t ps = (size_t) sysconf(_SC_PAGESIZE);
    unsigned char *base = mmap(NULL, 2 * ps, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS,
                               -1, 0);
    if (base == MAP_FAILED)
        return NULL;
    if (mprotect(base + ps, ps, PROT_NONE) != 0)
        return NULL;
    *out_pagesz = ps;
    /* Return a pointer such that exactly `n` bytes remain before the guard. */
    return base + ps - n;
}

/* ================= A. size coverage ================= */
/* The arm loads 8 bytes; the sibling arm only proves 1 byte is readable. */
__attribute__((noinline)) long size_cover(const void *p, int c) {
    if (c)
        return *(const long *) p;      /* 8-byte load — must NOT be speculated */
    return (long) *(const char *) p;   /* only proves 1 byte */
}

/* Same, with the narrow deref in the PRED instead of the sibling arm. */
__attribute__((noinline)) long size_cover_pred(const void *p, int c, long *sink) {
    *sink = (long) *(const char *) p;  /* pred proves 1 byte */
    if (c)
        return *(const long *) p;      /* 8-byte load — must NOT be speculated */
    return 0;
}

/* ================= B. truncating cast in the address key ============= */
__attribute__((noinline)) int trunc_key(const int *d, long big, int c, int *store_here) {
    /* Two DIFFERENT addresses: d[(int)big] and d[big]. On LP64 with
     * big = 2^32 + 1 the truncation yields 1 while the untruncated index is
     * 4294967297 — the key walk must not equate them. */
    int lo = d[(int) big];
    if (c) {
        *store_here = lo;
        return lo;
    }
    return lo + 1;
}

/* ================= C. scale injectivity ============================== */
__attribute__((noinline)) int scale_key(const int *d, int i, int c) {
    /* d[i] vs d[2*i]: same base, offsets i<<2 and i<<3. */
    if (c)
        return d[i];
    return d[2 * i];
}

__attribute__((noinline)) void scale_store(int *d, int i, int c, int v) {
    if (c)
        d[i] = v;
    else
        d[2 * i] = v;
}

/* ================= D. the shape that SHOULD still convert ============ */
/* Both arms deref the same address at the same width: legal to speculate. */
__attribute__((noinline)) float covered_both(const float *a, const float *s, int n, float *dst) {
    float acc = 0;
    for (int i = 0; i < n; i++) {
        /* a[i] is loaded on both paths -> full coverage -> convertible. */
        dst[i] = s[i] < 0 ? -a[i] : a[i];
        acc += dst[i];
    }
    return acc;
}

int main(void) {
    size_t ps;
    volatile int one = 1, zero = 0;

    /* ---- A: 8-byte load at the very end of a mapping ---- */
    {
        unsigned char *p = guarded(1, &ps);
        if (p) {
            *p = 0x5a;
            /* c == 0 takes the char arm; a speculated 8-byte load faults. */
            long r = size_cover(p, zero);
            CHECK(r == 0x5a, "size_cover: got %ld want 90\n", r);
            long sink = 0;
            long r2 = size_cover_pred(p, zero, &sink);
            CHECK(r2 == 0 && sink == 0x5a, "size_cover_pred: r=%ld sink=%ld\n", r2, sink);
        } else {
            printf("note: mmap guard unavailable, size-coverage probe skipped\n");
        }
    }

    /* ---- B: truncating cast must not alias two addresses ---- */
    {
        static int d[8];
        for (int i = 0; i < 8; i++)
            d[i] = 100 + i;
        int slot = -1;
        volatile long big = ((long) 1 << 32) + 1; /* (int)big == 1 */
        int got = trunc_key(d, big, one, &slot);
        CHECK(got == 101, "trunc_key: got %d want 101\n", got);
        CHECK(slot == 101, "trunc_key store: got %d want 101\n", slot);
    }

    /* ---- C: d[i] vs d[2*i] must stay distinct ---- */
    {
        static int d[16];
        for (int i = 0; i < 16; i++)
            d[i] = i * 7;
        volatile int vi = 3;
        CHECK(scale_key(d, vi, one) == 21, "scale_key true arm wrong\n");
        CHECK(scale_key(d, vi, zero) == 42, "scale_key false arm wrong\n");

        memset(d, 0, sizeof d);
        scale_store(d, vi, one, 111);
        CHECK(d[3] == 111 && d[6] == 0, "scale_store true arm: d[3]=%d d[6]=%d\n", d[3], d[6]);
        memset(d, 0, sizeof d);
        scale_store(d, vi, zero, 222);
        CHECK(d[6] == 222 && d[3] == 0, "scale_store false arm: d[3]=%d d[6]=%d\n", d[3], d[6]);
    }

    /* ---- D: the covered shape must still produce the right values ---- */
    {
        enum { N = 32 };
        static float a[N], s[N], dst[N], ref[N];
        float racc = 0;
        for (int i = 0; i < N; i++) {
            a[i] = (float) (i - 16) * 0.5f;
            s[i] = (float) (i % 3 - 1);
        }
        for (int i = 0; i < N; i++) {
            ref[i] = s[i] < 0 ? -a[i] : a[i];
            racc += ref[i];
        }
        float acc = covered_both(a, s, N, dst);
        for (int i = 0; i < N; i++)
            CHECK(dst[i] == ref[i], "covered_both[%d]: %g vs %g\n", i, dst[i], ref[i]);
        CHECK(acc == racc, "covered_both acc: %g vs %g\n", acc, racc);
    }

    printf("ifconv_speculation_adversarial: %s\n", fails ? "FAIL" : "OK");
    return fails != 0;
}
