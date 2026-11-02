/* General-CFG-cloner constant-stride coverage (review follow-up).
 *
 * The complete-unroll stride support has TWO cloners: the two-block cloner
 * (single-block bodies) and the general CFG cloner (multi-block bodies:
 * diamonds, nested loops, carried phis with multiple incoming edges).
 * `unroll_stride_complete.c` pins the stride arms, but every kernel there
 * has a single-block body -- only the two-block cloner runs. This file
 * routes stride loops through the GENERAL cloner, whose per-clone IV
 * substitution (`iv_init + t * iv_step`) and out-of-loop final value
 * (`iv_init + trip * iv_step`) must both scale by the stride: with the
 * stride guard lifted but the substitution unscaled, a stride-4 diamond
 * loop cloned iteration indices 0,1 instead of 0,4 and addressed the
 * wrong elements (caught in review; this test keeps it caught).
 *
 * Shapes: diamond body (both signs of stride), carried-accumulator phi
 * with two incoming edges, nested inner loop inside a strided outer
 * loop, and post-loop uses of the IV (the final-value substitution).
 * Results are printf'd and diffed against the oracle build.
 */
#include <stdio.h>

long la[16], lb[16];
volatile long vsink;

static void init(void) {
    for (int i = 0; i < 16; i++) {
        la[i] = i * 3 - 7;
        lb[i] = (i % 5) - 2;
    }
}

/* Diamond body, ascending stride 4: i = 0,4,8,12. Multi-block body:
 * the general cloner must substitute 0,4,8 for clones 1..3. */
static long diamond_up(void) {
    long s = 0;
    for (int i = 0; i < 16; i += 4) {
        if (la[i] > 0)
            s += la[i] * i;
        else
            s -= lb[i] * i;
    }
    return s;
}

/* Diamond body, descending stride 2: i = 14,12,...,2. */
static long diamond_down(void) {
    long s = 0;
    for (int i = 14; i > 0; i -= 2) {
        if (i & 2)
            s += la[i];
        else
            s -= la[i];
    }
    return s;
}

/* Carried accumulator phi defined in BOTH arms (two incoming back
 * edges): exercises the general cloner's prev_back threading across
 * clones with stride 3. i = 1,4,7,10 (non-zero init, ceil trip). */
static long carried_diamond(void) {
    long s = 5;
    for (int i = 1; i < 12; i += 3) {
        if (la[i] & 1)
            s = s + i * la[i];
        else
            s = s - i * lb[i];
    }
    return s;
}

/* Strided outer loop (multi-block body because of the inner loop):
 * i = 0,4. The inner unit-stride loop must survive intact in every
 * clone with the outer IV scaled. */
static long nested_stride(void) {
    long s = 0;
    for (int i = 0; i < 8; i += 4) {
        for (int j = 0; j < 4; j++)
            s += la[i + j] * (j + 1);
    }
    return s;
}

/* Post-loop IV uses: the final value substitution must be
 * `iv_init + trip * iv_step`, not `iv_init + trip`.
 * Loop 1: i = 2,5,8 -> exit 11.  Loop 2: i = 9,6,3 -> exit 0. */
static long post_iv(void) {
    long s = 0;
    int i;
    for (i = 2; i < 11; i += 3)
        s += la[i];
    s += i * 100;
    for (i = 9; i > 0; i -= 3)
        s += la[i];
    s += i;
    return s;
}

/* Post-loop IV use with a diamond body (final value through the
 * general cloner): i = 0,3,6,9 -> exit 12. */
static long post_iv_diamond(void) {
    long s = 0;
    int i;
    for (i = 0; i < 12; i += 3) {
        if (la[i] & 1)
            s += i;
        else
            s += 2 * i;
    }
    vsink = s;
    return i + (int)s;
}

/* `<=` exit with diamond body and residue: i = 0,5,10 -> exit 15. */
static long sle_diamond(void) {
    long s = 0;
    int i;
    for (i = 0; i <= 13; i += 5) {
        if (la[i] > 0)
            s += la[i];
        else
            s -= la[i];
    }
    return s + i;
}

int main(void) {
    init();
    printf("%ld %ld %ld %ld %ld %ld %ld\n",
           diamond_up(), diamond_down(), carried_diamond(), nested_stride(),
           post_iv(), post_iv_diamond(), sle_diamond());
    return 0;
}
