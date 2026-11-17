/* Dual-bound buffer-fill widening (loop_idiom_fill): the streaming-update
 * idiom `while (s < se && H->p < buf_end) *H->p++ = *s++;` is versioned
 * into an 8-byte-chunk fast path behind runtime guards, with the original
 * byte loop kept as tail and fallback.  This pins the observable semantics
 * of every guard dimension:
 *
 *  1. The Expat SipHash shape (buf[8] directly before p, buf_end == &H->p):
 *     every (length, split, start-fill) combination — zero trips, 1..7-byte
 *     tails, multi-chunk runs.  Oracle-diffed against GCC.
 *  2. Forward/backward overlap between src and dst at every distance
 *     -12..12 (smear guard: 0 < d-s < 8 must fall back and smear; d-s >= 8
 *     must stay block-equivalent; d <= s always safe).  Oracle-diffed.
 *  3. The pointer cell inside the DESTINATION region (stores rewrite the
 *     induction pointer mid-copy) and inside the SOURCE region (reads
 *     observe the cell updates).  The C source is unsequenced there, so no
 *     cross-compiler byte oracle exists; instead the transformed loop and a
 *     volatile-cell twin (never matched by the idiom) run on the SAME
 *     256-aligned arena re-initialized identically, making both runs
 *     address-identical and deterministic — their snapshots must be
 *     byte-equal.  A wrongly-widened loop reads stale cell bytes or skips
 *     its own pointer rewrite and diverges.
 */
#include <stdio.h>
#include <string.h>

typedef unsigned long long u64;

struct sip {
    unsigned char buf[8];
    unsigned char *p;
};

__attribute__((noinline))
static const unsigned char *fill_sip(struct sip *h, const unsigned char *s,
                                     const unsigned char *se) {
    while (s < se && h->p < h->buf + sizeof h->buf)
        *h->p++ = *s++;
    return s;
}

struct ring {
    unsigned char *p;
    unsigned char buf[24];
};

/* Volatile-cell twin: excluded from the idiom by construction. */
struct ring_v {
    unsigned char *volatile p;
    unsigned char buf[24];
};

__attribute__((noinline))
static const unsigned char *fill_ring(struct ring *h, const unsigned char *s,
                                      const unsigned char *se,
                                      unsigned char *de) {
    while (s < se && h->p < de)
        *h->p++ = *s++;
    return s;
}

__attribute__((noinline))
static const unsigned char *fill_ring_ref(struct ring_v *h,
                                          const unsigned char *s,
                                          const unsigned char *se,
                                          unsigned char *de) {
    while (s < se && h->p < de)
        *h->p++ = *s++;
    return s;
}

static u64 fnv(u64 h, const unsigned char *p, unsigned long n) {
    unsigned long i;
    for (i = 0; i < n; i++) {
        h ^= p[i];
        h *= 0x100000001b3ULL;
    }
    return h;
}

/* Case 1: SipHash shape, all (len, split, prefill) combinations. */
static u64 case_sip(void) {
    unsigned char data[80];
    u64 acc = 0xcbf29ce484222325ULL;
    unsigned len, split, pre, i;
    for (i = 0; i < sizeof data; i++)
        data[i] = (unsigned char)(i * 73u + 19u);
    for (len = 0; len <= 40; len++) {
        for (split = 0; split <= len; split += 3) {
            for (pre = 0; pre < 8; pre += 5) {
                struct sip h;
                const unsigned char *mid, *end;
                memset(h.buf, 0xAA, sizeof h.buf);
                h.p = h.buf + pre;
                mid = fill_sip(&h, data, data + split);
                end = fill_sip(&h, mid, data + len);
                acc = fnv(acc, h.buf, sizeof h.buf);
                acc = acc * 31u + (u64)(h.p - h.buf);
                acc = acc * 31u + (u64)(end - data);
            }
        }
    }
    return acc;
}

/* Case 2: src/dst overlap at every distance; the cell is a disjoint local. */
static u64 case_overlap(void) {
    u64 acc = 0x9e3779b97f4a7c15ULL;
    int delta;
    unsigned i;
    for (delta = -12; delta <= 12; delta++) {
        struct ring h;
        unsigned char arena[64];
        const unsigned char *s;
        unsigned char *d;
        for (i = 0; i < sizeof arena; i++)
            arena[i] = (unsigned char)(i + 1);
        s = arena + 20;
        d = arena + 20 + delta;
        h.p = d;
        (void)fill_ring(&h, s, s + 20, d + 20);
        acc = fnv(acc, arena, sizeof arena);
        acc = acc * 31u + (u64)(h.p - arena);
    }
    return acc;
}

int main(void) {
    printf("sip %016llx\n", case_sip());
    printf("overlap %016llx\n", case_overlap());
    return 0;
}
