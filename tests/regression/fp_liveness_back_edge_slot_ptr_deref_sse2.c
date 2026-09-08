#include <stdio.h>
/* Peephole FpLiveness regression (patE2): fold_ptr_deref_through_stack
 * rewrites `movq (%p), %rax; movq %rax, SLOT; …; movsd SLOT, %xmm` into a
 * direct `movsd (%p), %xmm` and deletes the slot store when the slot is
 * "dead".  The slot is d's home, re-read at the loop top on the next
 * iteration.  Expected 16 at every level. */
__attribute__((noinline)) double f(const long *bits, int n) {
    double d = 0.0, s = 0.0;
    for (int i = 0; i < n; i++) {
        s += d;
        __builtin_memcpy(&d, &bits[i], 8);
        s += d;
    }
    return s;
}
int main(void) {
    double v[4] = {1, 2, 3, 4};
    long b[4];
    __builtin_memcpy(b, v, sizeof b);
    printf("%g\n", f(b, 4));
    return 0;
}
