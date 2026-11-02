/* RA-GLA-03 / back-edge safety pin.
 *
 * A source-less global address that feeds a loop-latch phi can only ever
 * be rematerialized ALONG FORWARD EDGES: the GLA materializer must never
 * splice a reload trampoline into a back edge (the latch edge already
 * carries the phi incoming whose weight the policy prices at >= 10 by
 * construction, above the remat weight cap, and the one-live-segment cap
 * rejects the weaving multi-segment form).
 *
 * This file builds a genuine latch phi: the same global base `gbuf` is
 * re-defined on the edge that loops back (`q = p` before the `goto loop`)
 * and a *different* p-derived value enters on the forward/false edge, so
 * the edge-splitting/trampoline machinery is put directly in front of a
 * back edge. Even with every ADMITTING knob forced maximally open
 * (reach=999, segment cap=1024, weight cap=64) the plan must contain
 * ZERO trampolines; check_gla_backedge_no_trampoline.sh asserts that and
 * requires forced-on/forced-off runtime equivalence.
 */
#include <stdint.h>
#include <stdio.h>

static char gbuf[64] = "ABCDEFGH-backedge-trampoline!!";

/* Defined out-of-line so the optimizer cannot fold the latch decision. */
extern int opaque(int);
__attribute__((noinline))
int opaque(int x) {
    static volatile int z;
    z += x;
    return z & 1;
}

__attribute__((noinline))
static int loop_back(int n, uint32_t a, uint32_t b, uint32_t d,
                     uint32_t e, uint32_t f, uint32_t g,
                     uint32_t h, uint32_t i, uint32_t j) {
    char *p0 = gbuf;
    /* Many independent live values create real register pressure around
       the latch, the regime in which an edge trampoline would tempt. */
    uint32_t x0 = a * b + 1u, x1 = d * e + 2u, x2 = f * g + 3u;
    uint32_t x3 = h * i + 4u, x4 = j * a + 5u, x5 = b ^ d ^ f ^ h;
    uint32_t x6 = e + g + i + j, x7 = a * g + b * h;
    uint32_t x8 = a + d + j, x9 = b * e + h, x10 = f ^ i ^ j, x11 = g * h + a;
    int sum = 0, k = 0;
    char *q = p0;
loop:
    sum += q[k & 3];
    if (++k >= n) goto out;
    char *p = gbuf;
    q = p;                        /* true/back edge carries p into the phi */
    if (opaque(k)) goto loop;     /* L -> H: genuine back edge, phi incoming */
    q = p + 4;                    /* forward/false edge: other phi incoming */
    goto loop;
out:
    /* Keep every pressure live through the exit so they span the loop. */
    return sum
        + (int)(x0 ^ x1 ^ x2 ^ x3 ^ x4 ^ x5 ^ x6 ^ x7 ^ x8 ^ x9 ^ x10 ^ x11)
        + (int)(x0 + x1 + x2 + x3 + x6 + x8 + x10)
        + (int)(x4 * 3u + x5 * 5u + x7 * 9u + x9 * 11u + x11 * 13u);
}

int main(void) {
    int s = 0;
    for (int n = 7; n <= 40; n += 3)
        s += loop_back(n, 11u, 13u, 17u, 19u, 23u, 29u, 31u, 37u, 41u);
    printf("%d\n", s & 0xffff);
    return 0;
}
