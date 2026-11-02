/* Triangular-index strength reduction (quadratic_sr): for non-overflowing
 * t, the carried-accumulator form must match the direct t*(t+1)/2 + i + 1
 * computation bit-for-bit. Differential-checked against gcc by the harness.
 */
#include <stdio.h>
static long long acc_direct(void) {
    long long acc = 0;
    for (int i = 0; i < 200; i++)
        for (int j = 0; j < 200; j++) {
            int t = i + j;
            acc += (long long)(t * (t + 1) / 2 + i + 1);
        }
    return acc;
}
int main(void) {
    printf("%lld\n", acc_direct());
    return 0;
}
