/* Regression: GCC vector-extension comparisons and unary ops must be
 * lowered element-wise, not against the vector's stack address.
 *
 * Prior defect: the frontend routed vector comparisons (`<=`,`>`,`==`,...) and
 * unary vector ops (`~`,`-`) through the generic *scalar* path, which lowered
 * a vector operand to its stack *address*.  `~(V){0} <= 0` became
 * `~(ptr <= 0)` and the downstream memcpy/load dereferenced the bit-negated
 * address as a pointer -> SIGSEGV.  (gcc PR 110817, 105613, 109040, 94412.)
 *
 * Each sub-case aborts if LCCC miscompiles the per-lane mask / vector math.
 * The assertions are copied from the upstream GCC tests that this fix closes. */
#include <stdlib.h>

/* PR 110817: 1-lane unsigned long vector comparison then bitnot. */
static void t_110817(void) {
    typedef unsigned long __attribute__((__vector_size__(8))) V;
    V v = ~((V) { 0 } <= 0);
    if (v[0])
        __builtin_abort();
    V w = ((V) { 5 } > 0);
    if (!w[0])
        __builtin_abort();
    V z = ((V) { 5 } == 0);
    if (z[0])
        __builtin_abort();
    V n = ((V) { 5 } != 0);
    if (!n[0])
        __builtin_abort();
}

/* PR 105613: 16-byte __int128 vector compare != 0. */
static void t_105613(void) {
    typedef unsigned __int128 __attribute__((__vector_size__(16))) V;
    V mask = ((V) { 5 } != (V) { 0 });
    if (mask[0] != ~(unsigned __int128) 0)
        __builtin_abort();
    mask = ((V) { 0x500000005ULL } != (V) { 0 });
    if (mask[0] != ~(unsigned __int128) 0)
        __builtin_abort();
    mask = ((V) { 0 } != (V) { 0 });
    if (mask[0] != 0)
        __builtin_abort();
}

/* PR 109040: 16-lane unsigned short compare against a masked vector. */
static unsigned short a, b, c, d; /* zero-initialised globals, as in PR 109040 */

static void t_109040(void) {
    typedef unsigned short __attribute__((__vector_size__(32))) V;
    V m = (V) { 0, 15 };
    V v = 6 > ((V) { 2124, 8 } & m);
    unsigned short uc = v[0] + a + b + c + d;
    if (uc != (unsigned short) ~0)
        __builtin_abort();
}

/* PR 94412: vector negation then division is element-wise. */
static void t_94412(void) {
    typedef unsigned V __attribute__((__vector_size__(sizeof(unsigned) * 2)));
    V a = (V) { 1, 0 };
    V b = (V) { 3, 0x7fffffffU };
    /* foo: *w = -*v / 11 */
    V c = -a / 11;
    if (c[0] != -1U / 11 || c[1] != 0)
        __builtin_abort();
    /* bar: *w = -18 / -*v */
    V d = -18 / -b;
    if (d[0] != -18U / -3U || d[1] != -18U / -0x7fffffffU)
        __builtin_abort();
}

int main(void) {
    t_110817();
    t_105613();
    t_109040();
    t_94412();
    return 0;
}
