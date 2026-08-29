#include <stdio.h>
int side(int x) { return x; }
/* Agent Z bool_thread soundness repro: the merge block holds the phi, a
 * merge-local Cmp whose dest is used AFTER the branch (q), and branches on
 * the PHI. Threading the merge would delete the Cmp and dangle q's use.
 * Rule-3 relaxation (Cmp-LHS use of a merge phi) admits this candidate, so
 * the bool-shape path must reject it unless the Cmp is provably dead. */
int target(int c, int a, int b) {
    int p;
    if (c) p = a; else p = b;
    int q = (p < 5);
    if (p) side(1); else side(2);
    return q * 10 + side(3);
}
int main(void) {
    int sum = 0;
    for (int i = -2; i <= 2; i++)
        for (int j = -2; j <= 2; j++)
            sum += target(i, j, -j);
    printf("%d\n", sum);
    return 0;
}
