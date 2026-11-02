/* BitTest(base, 0) is `base & 1`, not zero.
 *
 * lccc canonicalizes `(base >> index) & 1` into a cross-target `BitTest`
 * (src/passes/simplify.rs), and x86 lowers it with BT. The simplifier used to
 * fold a *zero index* to the constant 0 — as if `x >> 0` meant "shift nothing
 * away, so nothing is left" — when in fact `(x >> 0) & 1` is bit zero of x,
 * i.e. 1 for every odd x. With a non-constant base the fold produced a wrong
 * constant and, in a diamond, sent the branch down the wrong arm:
 *
 *     if (((x >> 0) & 1) ^ 1) { ... } else { ... }   // took the then-arm
 *
 * for odd x. Found by scripts/gen_slot_stress.py (seed 1, -O2); the volatile
 * load keeps the base out of reach of constant propagation so the simplifier
 * is the pass under test.
 */
#include <stdio.h>

static volatile unsigned long long gv;

int main(void)
{
    gv = 0xacc0cec466c569cfULL;   /* bits 0..7 = 1,1,1,1,0,0,1,1  (odd) */
    unsigned long long x = gv;

    int b0 = (int)((x >> 0u) & 1u);
    int b1 = (int)((x >> 1u) & 1u);
    int b5 = (int)((x >> 5u) & 1u);
    int b7 = (int)((x >> 7u) & 1u);

    if (b0 != 1) { printf("FAIL b0=%d (want 1)\n", b0); return 1; }
    if (b1 != 1) { printf("FAIL b1=%d (want 1)\n", b1); return 2; }
    if (b5 != 0) { printf("FAIL b5=%d (want 0)\n", b5); return 3; }
    if (b7 != 1) { printf("FAIL b7=%d (want 1)\n", b7); return 4; }

    /* The branch form: a folded constant must not pick the wrong arm. */
    if (((x >> 0u) & 1u) ^ 1) {
        printf("FAIL branch: ((x>>0)&1)^1 is false for odd x\n");
        return 5;
    }

    /* Even base: bit zero must fold the other way. */
    gv = 0xacc0cec466c569ceULL;
    unsigned long long y = gv;
    if (((y >> 0u) & 1u) != 0) { printf("FAIL even: bit0=%d\n", (int)((y >> 0u) & 1u)); return 6; }
    if (!(((y >> 0u) & 1u) ^ 1)) { printf("FAIL even branch\n"); return 7; }

    printf("PASS bittest_index_zero\n");
    return 0;
}
