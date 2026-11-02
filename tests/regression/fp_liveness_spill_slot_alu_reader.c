#include <stdio.h>
/* Peephole FpLiveness regression (t2): eliminate_fp_spill_around_load
 * matched `movsd %xmm0, SLOT … movsd X, %xmm0 … addsd SLOT, %xmm0` while a
 * `mulsd SLOT, %xmm0` READ the slot and a `movq %rax, SLOT` REDEFINED it in
 * between (neither is a LoadXmmRbp/StoreXmmRbp, so the old scan saw
 * neither).  Expected "-1.5 0.5 -24". */
int main(void) {
    double p[8];
    for (int i = 0; i < 8; i++) { p[i] = i * 0.5 - 3; }
    double s = 0;
    for (int i = 0; i < 8; i++) s += p[i] * (i + 1);
    printf("%g %g %g\n", p[3], p[7], s);
    return 0;
}
