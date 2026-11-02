/* Synthetic isolated calls, not an application benchmark. The dedicated
 * driver compiles this TU separately from the kernels (no LTO), so constant
 * results do not allow the harness to delete calls. Standalone corpus use
 * deliberately has a separate inclusion fallback. */
#include <stdint.h>
#include <stdio.h>
#ifndef COUNT_OBSERVATIONS_EXTERNAL
#define COUNT_OBSERVATIONS_KERNEL_ONLY
#include "../../regression/count_observations.c"
#else
extern int co_cap64(uint64_t, uint64_t);
extern int co_zero64(uint64_t, uint64_t);
extern int co_nonzero64(uint64_t, uint64_t);
#endif
#ifndef ROUNDS
#define ROUNDS 134217728u
#endif
#ifndef COUNT_CASE
#define COUNT_CASE 0
#endif
int main(void) {
    uint64_t state=0x9e3779b97f4a7c15ull, sum=0;
    for(unsigned i=0;i<ROUNDS;++i) {
        state^=state<<13;state^=state>>7;state^=state<<17;
        uint64_t x=(i&3u) ? state:((uint64_t)1<<(i&63u));
#if COUNT_CASE == 0
        sum+=(unsigned)co_cap64(x,0);
#elif COUNT_CASE == 1
        sum+=(unsigned)co_zero64((i&63u) ? x:0,0);
#elif COUNT_CASE == 2
        sum+=(unsigned)co_nonzero64(x,0);
#else
#error unsupported count observation benchmark
#endif
    }
    printf("%016llx\n",(unsigned long long)sum);
    return 0;
}
