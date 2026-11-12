/* Differential fixture for the fused 16-byte `Load; Store` pair.
 *
 * A 16-byte integer carrier (`__int128`/`unsigned __int128`, and any 16-byte
 * aggregate the front end lowers to one) is not register-homed, so a copy is
 *
 *     %v = load  *src        -> movdqu  *src, %xmm0 ; movdqu %xmm0, slot
 *     store %v -> *dst       -> movdqu  slot, %xmm0 ; movdqu %xmm0, *dst
 *
 * when the pair is adjacent in its block.  The backend now copies the bytes
 * directly (`movdqu *src, %xmm0 ; movdqu %xmm0, *dst`).  This fixture is the
 * semantic half of the gate: every shape that reaches the fused path, plus
 * the shapes that must NOT fuse and must stay exact.
 *
 * Shapes covered:
 *   adjacent      `*dst = *src`                       (fuses)
 *   loop          the same, in a loop with a varying index (fuses per iter)
 *   aliasing      `dst == src` and one-byte overlaps   (order must survive)
 *   nonadjacent   the store pointer is computed between the load and the
 *                 store, so the pair is not fused: the slot path must still
 *                 be exact
 *   chain         a copy whose result is copied again (single-use each)
 *   narrow        copies whose halves are read afterwards (a use that is not
 *                 the store, so the value keeps its slot)
 *
 * The checksum comes from GCC 16.2 on the same source.  The shape half (that
 * the two-instruction sequence is actually emitted, with and without a
 * segment prefix) lives in `tests/corpus/test_i128_copy_fusion.py`.
 */
#include <stddef.h>
#include <stdio.h>

#define N 64
#define EXPECTED 11950421421446008775ULL

typedef unsigned __int128 u128;
/* A 16-byte access with no alignment requirement and no aliasing promise:
 * the overlap test below addresses the same object at odd offsets, which a
 * plain `u128 *` may not do (GCC 16.2 at -O2 emitted an aligned move for it
 * and the fixture segfaulted). */
typedef u128 u128_unaligned __attribute__((aligned(1), may_alias));
typedef struct { unsigned long long lo, hi; } pair;

static u128 src[N];
static u128 dst[N];
/* Declared as `u128` so the aliasing test below writes each 16-byte group
 * through its own real type: forming a `u128 *` from a `char` array is
 * strict-aliasing UB, and GCC 16.2 at -O2 folds the copy away and runs off
 * the end of the object (the fixture segfaulted before this). */
static u128 words[4 * N + 4];
#define bytes ((unsigned char *) words)

static void adjacent(void) {
    for (int i = 0; i < N; i++) dst[i] = src[i];
}

/* The store pointer is computed between the load and the store: the pair is
 * not adjacent in its block, so the value's slot is the transport. */
static void nonadjacent(void) {
    for (int i = 0; i < N; i++) {
        u128 v = src[i];
        u128 *p = &dst[N - 1 - i];
        *p = v;
    }
}

/* dst == src and partial overlap: the fused copy performs the load first and
 * the store second, exactly like the unfused pair, so the result is the same
 * bytes either way. */
static void aliasing(void) {
    for (int i = 0; i <= 16; i++) {
        u128_unaligned *p = (u128_unaligned *) (void *) (bytes + i);
        *p = *p;                      /* dst == src */
    }
    for (int i = 0; i < 16; i++) {
        u128_unaligned *d = (u128_unaligned *) (void *) (bytes + i + 1);
        u128_unaligned *s = (u128_unaligned *) (void *) (bytes + i);
        *d = *s;                      /* 15 bytes of overlap */
    }
}

/* A copy chain: each value has exactly one use (the next store), so the first
 * two pairs fuse and the last store's source is the phi-carried value. */
static void chain(void) {
    u128 t = src[0];
    dst[0] = t;
    t = dst[0];
    dst[1] = t;
}

/* The halves are read: the copy's value has a second use, so it stays
 * slot-homed and the pair must not fuse. */
static unsigned long long narrow(void) {
    unsigned long long acc = 0;
    for (int i = 0; i < N; i++) {
        u128 v = src[i];
        dst[i] = v;
        acc += (unsigned long long) v + (unsigned long long) (v >> 64);
    }
    return acc;
}

static unsigned long long fnv(unsigned long long h, unsigned long long v) {
    for (int i = 0; i < 8; i++) {
        h ^= (v >> (i * 8)) & 0xff;
        h *= 1099511628211ULL;
    }
    return h;
}

int main(void) {
    for (int i = 0; i < N; i++)
        src[i] = ((u128) (i * 0x9e3779b97f4a7c15ULL + 1) << 64) | (unsigned) (i * 2654435761u + 7);
    for (int i = 0; i < 4 * N + 4; i++) bytes[i] = (unsigned char) (i * 37 + 11);

    adjacent();
    nonadjacent();
    chain();
    aliasing();
    unsigned long long acc = narrow();

    unsigned long long h = 1469598103934665603ULL;
    for (int i = 0; i < N; i++) {
        h = fnv(h, (unsigned long long) dst[i]);
        h = fnv(h, (unsigned long long) (dst[i] >> 64));
    }
    for (int i = 0; i < 4 * N + 4; i++) h = fnv(h, bytes[i]);
    h = fnv(h, acc);

    printf("%llu\n", h);
    return h == EXPECTED ? 0 : 1;
}
