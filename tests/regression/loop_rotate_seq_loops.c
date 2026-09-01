/* PF-17: two sequential counted loops in one function.
 *
 * Rotation of the SECOND loop must take its self-loop-phi init incoming
 * from the GUARD (header), not the original preheader label. The first
 * loop's preheader is the entry and accidentally becomes a predecessor
 * after header-merge, so the bug hid there; the second loop's preheader
 * is a jump block that cfg_simplify then kills, collapsing the phi to a
 * Copy of the latch operand (use-before-def, garbage IV).
 *
 * Companion .env sets CCC_LOOP_ROTATE=1. Differential vs GCC -O2.
 */
#include <stdio.h>

int main(void) {
    int a[8];
    int s = 0;
    int i;
    for (i = 0; i < 8; i++)
        a[i] = i + 1;
    for (i = 0; i < 8; i++)
        s += a[i];
    /* 1+2+...+8 = 36 */
    printf("%d\n", s);
    return s != 36;
}
