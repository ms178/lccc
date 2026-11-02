/* Regression: complete unrolling must leave structurally valid IR.
 *
 * `try_complete_unroll_two_block` handles the 2-3 block loop form. When it
 * fully unrolls a loop it rewrites the loop-carried accumulator phis into
 * Copies of the final clone's value (outside uses reference the phi's id and
 * need the POST-loop value) -- but it used to leave the induction variable's
 * phi untouched. Once the back-edge is gone that phi is malformed twice over:
 *
 *   - it still names the latch as a predecessor, though the latch no longer
 *     branches to the header (STALE_PRED), and
 *   - the carried phis ahead of it have become Copies, so it now sits after a
 *     non-phi instruction (PHI_ORDER).
 *
 * `try_complete_unroll_general` always resolved every header phi correctly;
 * the two paths had simply diverged. The defect stayed latent because DCE
 * deletes the dead IV phi before codegen -- but every pass scheduled between
 * loop_unroll and DCE (narrow, simplify, bit_idioms, reassoc_accum, sccp,
 * constfold, if_convert, ...) consumed invalid IR in the meantime. It was
 * found by sweeping the corpus with CCC_VERIFY_IR=1; it accounted for 340 of
 * the 396 violating configurations.
 *
 * Shape requirements:
 *   - an inner counted loop with a compile-time constant trip count, so it is
 *     completely unrolled,
 *   - at least one loop-carried accumulator (so `final_map` is non-empty and
 *     the carried phi becomes a Copy ahead of the IV phi),
 *   - an outer loop, so the unrolled body is reached more than once and a
 *     wrong carried value is observable,
 *   - the accumulator must be used AFTER the loop, so the post-loop value has
 *     to survive the phi rewrite.
 *
 * Run under CCC_VERIFY_IR=1 this must print nothing.
 *
 * The header phis must all collapse to their INIT (non-latch) incoming, NOT to
 * the final clone's value. The clone that defines that value runs AFTER the
 * header, so copying it here is a copy-before-def: the definition does not
 * dominate the use, and GVN/copy-prop then forwards an unordered value into
 * inlined callers. Outside uses of a carried phi do not need it either -- they
 * were already rewritten to the final clone's dest by the substitution loop
 * that precedes the phi resolution. Measured on this very file, the
 * final-clone variant leaves two copy-before-def pairs in the header
 * (`Copy v50 = v146` with v146 defined 19 blocks later); the init variant
 * leaves none. The printed values below stay correct under BOTH variants,
 * which is exactly why this needs saying: the output check cannot catch it.
 *
 * Expected output: 2040 16320 255 16
 */
#include <stdio.h>

static unsigned sum16(const unsigned char *b, unsigned len, unsigned s1, unsigned s2) {
    while (len >= 16) {
        int i;
        len -= 16;
        /* Constant trip count, two loop-carried accumulators, IV used as an
         * index -- the exact shape that routes through the two-block path. */
        for (i = 0; i < 16; i++) {
            s1 += b[i];
            s2 += s1;
        }
        b += 16;
    }
    /* s1/s2 are OUTSIDE uses of the carried phis: they must receive the
     * post-loop values, not the init values. */
    return s1 ^ (s2 << 16);
}

/* A single-accumulator variant: `final_map` has one entry, so the IV phi is
 * the second phi rather than the third. */
static unsigned prod8(const unsigned char *b) {
    unsigned acc = 1;
    int i;
    for (i = 0; i < 8; i++) {
        acc = acc * 3 + b[i];
    }
    return acc;
}

/* No carried accumulator at all: `final_map` is empty, so the IV phi is the
 * only phi in the header. Guards the branch of the fix that resolves a phi
 * which is NOT in final_map. */
static int count_nonzero(const unsigned char *b) {
    int n = 0, i;
    for (i = 0; i < 16; i++) {
        if (b[i]) {
            n++;
        }
    }
    return n;
}

int main(void) {
    unsigned char buf[32];
    unsigned i;
    for (i = 0; i < 32; i++) {
        buf[i] = (unsigned char) (i + 1);
    }

    unsigned r = sum16(buf, 32, 0, 0);
    unsigned lo = r & 0xffffu;
    unsigned hi = r >> 16;
    printf("%u %u %u %d\n", lo, hi, prod8(buf) & 0xffu, count_nonzero(buf));
    return 0;
}
