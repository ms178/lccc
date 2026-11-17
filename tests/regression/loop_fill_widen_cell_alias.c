/* Dual-bound buffer-fill widening (loop_idiom_fill): cell-aliasing guards.
 *
 * When the memory cell holding the destination pointer lies INSIDE the
 * store region, the byte loop rewrites its own induction pointer mid-copy;
 * inside the source region, it reads back its own cell updates.  The
 * widened 8-byte path must refuse both at runtime (cell-vs-dst and
 * cell-vs-src guards) and fall back to the byte loop.
 *
 * The C source for these shapes has unsequenced accesses, so compilers may
 * legitimately order the cell bump and the data access differently — GCC
 * is NOT a valid oracle here (and under GCC's possible ordering the walk
 * is not even contained).  Under lccc's fixed lowering (cell store, then
 * data access) both constructions below are deterministic and stay inside
 * a 256-aligned arena.  The check is in-process: the transformed loop and
 * a volatile-cell twin (never matched by the idiom) run on the SAME arena,
 * re-initialized identically, so both runs are address-identical; their
 * snapshots must be byte-equal.  A wrongly-widened loop reads stale cell
 * bytes or skips its own pointer rewrite and diverges.  Exit 1 on any
 * mismatch. */
#include <stdio.h>
#include <string.h>

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

static _Alignas(256) union {
    struct ring r;
    struct ring_v v;
    unsigned char raw[sizeof(struct ring)];
} arena;

int main(void) {
    unsigned char src[32];
    unsigned char snap_t[sizeof arena.raw], snap_r[sizeof arena.raw];
    long off_t_, off_r_;
    unsigned i;
    int ok = 1;

    /* Source bytes keep any rewritten low pointer byte inside the
     * 256-aligned arena block, so both runs stay contained and halt. */
    for (i = 0; i < sizeof src; i++)
        src[i] = (unsigned char)(8 + (i * 5) % 24);

    /* Cell inside DST: p points at its own cell; the first store rewrites
     * the pointer's low byte and teleports the walk into the buffer. */
    memset(arena.raw, 0x11, sizeof arena.raw);
    arena.r.p = arena.raw;
    (void)fill_ring(&arena.r, src, src + 16, arena.raw + sizeof arena.raw);
    off_t_ = (long)(arena.r.p - arena.raw);
    memcpy(snap_t, arena.raw, sizeof snap_t);

    memset(arena.raw, 0x11, sizeof arena.raw);
    arena.v.p = arena.raw;
    (void)fill_ring_ref(&arena.v, src, src + 16, arena.raw + sizeof arena.raw);
    off_r_ = (long)(arena.v.p - arena.raw);
    memcpy(snap_r, arena.raw, sizeof snap_r);
    ok &= off_t_ == off_r_ && memcmp(snap_t, snap_r, sizeof snap_t) == 0;
    printf("cell-in-dst %s\n", ok ? "ok" : "MISMATCH");

    /* Cell inside SRC: the copied window covers the pointer cell, so the
     * byte loop's reads observe its own cell updates. */
    memset(arena.raw, 0x22, sizeof arena.raw);
    arena.r.p = arena.r.buf;
    (void)fill_ring(&arena.r, arena.raw, arena.raw + 20,
                    arena.raw + sizeof arena.raw);
    off_t_ = (long)(arena.r.p - arena.raw);
    memcpy(snap_t, arena.raw, sizeof snap_t);

    memset(arena.raw, 0x22, sizeof arena.raw);
    arena.v.p = arena.v.buf;
    (void)fill_ring_ref(&arena.v, arena.raw, arena.raw + 20,
                        arena.raw + sizeof arena.raw);
    off_r_ = (long)(arena.v.p - arena.raw);
    memcpy(snap_r, arena.raw, sizeof snap_r);
    ok &= off_t_ == off_r_ && memcmp(snap_t, snap_r, sizeof snap_t) == 0;
    printf("cell-in-src %s\n", ok ? "ok" : "MISMATCH");

    return ok ? 0 : 1;
}
