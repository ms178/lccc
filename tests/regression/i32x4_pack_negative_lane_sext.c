/* Regression: the four-lane i32 pack must not sign-extend lane 2 into lane 3.
 *
 * SLP vectorises `out[i] = ia[i] + ia[3 - i]` into a VecPackI32x4 of four
 * scalars followed by a 128-bit add.  The pack combines lanes 2 and 3 in one
 * 64-bit GPR (`x2 | (x3 << 32)`).  Loading lane 2 as a sign-extended 64-bit
 * value filled the high dword with ones whenever lane 2 was negative, so lane 3
 * came out as `x3 + (-1)`.  Here `ia[1] == -2` is lane 2 of the pack, so lane 3
 * (`ia[3] + ia[0]`) was 39 instead of 41.  The bug is in the SSE2 baseline
 * and does not depend on any AVX flag.
 *
 * Every element is printed and compared with the scalar reference, so any
 * lane-level corruption fails the run.
 */
#include <stdio.h>

__attribute__((noinline)) static void reverse_sum(const int *ia, int *out)
{
    for (int i = 0; i < 4; i++) {
        out[i] = ia[i] + ia[3 - i];
        out[4 + i] = ia[i] * 8;
    }
}

int main(void)
{
    /* Negative lanes in both the low (x2) and high (x3) pack positions. */
    const int ia[4] = {1, -2, 3, 40};
    int got[8], exp[8];
    int fails = 0;

    reverse_sum(ia, got);
    for (int i = 0; i < 4; i++) {
        exp[i] = ia[i] + ia[3 - i];
        exp[4 + i] = ia[i] * 8;
    }
    for (int k = 0; k < 8; k++) {
        if (got[k] != exp[k]) {
            printf("lane %d: got %d expected %d\n", k, got[k], exp[k]);
            fails++;
        }
    }

    /* A second pattern with a negative value in the high lane too. */
    const int ib[4] = {-5, 7, -9, -11};
    int got2[8], exp2[8];
    reverse_sum(ib, got2);
    for (int i = 0; i < 4; i++) {
        exp2[i] = ib[i] + ib[3 - i];
        exp2[4 + i] = ib[i] * 8;
    }
    for (int k = 0; k < 8; k++) {
        if (got2[k] != exp2[k]) {
            printf("pattern2 lane %d: got %d expected %d\n", k, got2[k], exp2[k]);
            fails++;
        }
    }

    if (fails) {
        printf("FAILURES: %d\n", fails);
        return 1;
    }
    printf("PASS\n");
    return 0;
}
