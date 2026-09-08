#include <stdio.h>
#include <math.h>
/* Peephole FpLiveness regression (hoistH): promote_loop_invariant_fp_load
 * parked the invariant `a` in %xmm2 for the loop, but the body calls
 * `sin` — every XMM register is caller-saved, so %xmm2 was destroyed on
 * each iteration.  With -mno-avx eliminate_fp_spill_around_load also
 * relayed through %xmm1 across the same call.  Expected 11.637190. */
__attribute__((noinline)) double f(double a, int n) {
    double s = 0;
    for (int i = 0; i < n; i++) { double t = a; s += sin(t) + t; }
    return s;
}
int main(void) { printf("%.6f\n", f(2.0, 4)); return 0; }
