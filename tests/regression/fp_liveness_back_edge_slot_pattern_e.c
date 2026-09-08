#include <stdio.h>
/* Peephole FpLiveness regression (patE): eliminate_fp_xmm_roundtrips
 * Pattern E drops `movq %rax, SLOT` when `movq %rax, %xmm0` follows and the
 * slot is "dead".  The old textual forward scan could not see the reader at
 * the loop TOP (reached through the back edge): `s += d` on the next
 * iteration reloads d's home slot.  Expected 27. */
__attribute__((noinline)) double f(const double *a, int n) {
    double d = 1.0, s = 0.0;
    for (int i = 0; i < n; i++) {
        s += d;
        d = a[i];
        s += d * 2.0;
    }
    return s;
}
int main(void) {
    double a[4] = {1, 2, 3, 4};
    printf("%g\n", f(a, 4));
    return 0;
}
