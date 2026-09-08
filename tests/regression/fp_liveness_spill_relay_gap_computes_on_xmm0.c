#include <stdio.h>
#include <math.h>
/* Peephole FpLiveness regression (f3): eliminate_fp_spill_around_load's
 * role swap (`K` loads into %xmm1, `L` adds %xmm1) is only valid when the
 * gap between K and L never touches %xmm0.  Here `roundsd` computes on
 * %xmm0 between K and L; the swap made it round sqrt(a) instead of a.
 * The pass must relay via `movapd %xmm0, %xmm1` instead.  Expected
 * 0x1.8c65f050918f1p+4 (= sqrt(7.7)+7+8+7). */
__attribute__((noinline)) double f3(double a) { return sqrt(a) + floor(a) + ceil(a) + trunc(a); }
int main(void) { printf("%a\n", f3(7.7)); return 0; }
