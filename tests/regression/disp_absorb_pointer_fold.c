/* A constant pointer adjustment feeding one memory consumer must fold into
 * the consumer's displacement — and only when every side condition holds.
 *
 * The x86 peephole `disp_absorb` rewrites
 *
 *     addq $256, %rdi
 *     vfmadd132ps (%rdi), %ymm1, %ymm0
 * into
 *     vfmadd132ps 256(%rdi), %ymm1, %ymm0
 *
 * when the adjusted register dies at the consumer and the flags written by
 * the `addq` are dead.  The traps this test springs if the fold goes wrong:
 *
 *  - at(): the motivating shape — a wrong combined displacement reads the
 *    neighbouring element;
 *  - at_neg(): a NEGATIVE adjustment, exercising the signed displacement
 *    arithmetic (subq and below-base offsets);
 *  - keep_alive(): the register survives the consumer (stored through an
 *    out-parameter), so folding would hand the caller an un-adjusted
 *    pointer;
 *  - flags_used(): the adjustment's flags ARE the branch condition, so
 *    deleting the `addq` would branch on stale flags.
 */
#include <stdio.h>

__attribute__((noinline)) float at(const float *p) { return p[64]; }

__attribute__((noinline)) float at_neg(const float *p) { return p[-16]; }

__attribute__((noinline)) float keep_alive(const float *p,
                                           const float **res) {
    const float *q = p + 64;
    *res = q;
    return *q;
}

__attribute__((noinline)) long flags_used(long x) {
    long y = x + 3;
    if (y == 0)
        return -1;
    return y;
}

int main(void) {
    static float buf[160];
    for (int i = 0; i < 160; i++)
        buf[i] = (float)i * 0.5f;

    int bad = 0;
    if (at(buf) != 32.0f) bad |= 1;
    if (at_neg(buf + 80) != 32.0f) bad |= 2;

    const float *q = 0;
    if (keep_alive(buf, &q) != 32.0f || q != buf + 64) bad |= 4;

    if (flags_used(-3L) != -1L) bad |= 8;
    if (flags_used(7L) != 10L) bad |= 16;

    printf("%g %g %d bad=%d\n", (double)at(buf), (double)at_neg(buf + 80),
           (int)(q - buf), bad);
    return bad;
}
