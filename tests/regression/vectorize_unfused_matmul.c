/* Two-rounding packed multiply/add, including every tail length 0..65.
 * (1+2^-27)*(1-2^-27) rounds to 1; adding -1 gives +0. An FMA gives
 * -2^-54 instead. Hash every element, including both boundary canaries.
 */
#include <stdint.h>
#include <stdio.h>

double factor;
static double B[67], C[67];

__attribute__((noinline))
double update(int n, double live_across_loop)
{
    for (int j = 0; j < n; ++j)
        C[j + 1] += factor * B[j + 1];
    return live_across_loop;
}

int main(void)
{
    uint64_t hash = UINT64_C(1469598103934665603);
    factor = 1.0 + 0x1p-27;
    for (int n = 0; n <= 65; ++n) {
        for (int j = 0; j < 67; ++j) {
            B[j] = (j & 1) ? 1.0 - 0x1p-27 : 1.0 + 0x1p-27;
            C[j] = -1.0;
            if (j % 11 == 0) { B[j] = 0.0; C[j] = -0.0; }
            if (j % 11 == 1) { B[j] = 0x0.0000000000001p-1022; C[j] = 0.0; }
        }
        C[0] = 12345.0;
        C[66] = -54321.0;
        union { double d; uint64_t u; } live;
        live.d = update(n, (double)n + 0.25);
        hash = (hash ^ live.u) * UINT64_C(1099511628211);
        for (int j = 0; j < 67; ++j) {
            union { double d; uint64_t u; } bits;
            bits.d = C[j];
            hash = (hash ^ bits.u) * UINT64_C(1099511628211);
        }
    }
    printf("%016llx\n", (unsigned long long)hash);
    return 0;
}
