/* Synthetic throughput bundle, not a workload-speed claim. Keep the exact
 * noinline entry points shared with the native semantic/mechanism fixture. */
#define BATCH6_BENCH 1
#include "../../regression/llvm_missed_batch6.c"
#include <stdio.h>

#ifndef BATCH6_SCALAR_TRIPS
#define BATCH6_SCALAR_TRIPS 4000000u
#endif

int main(void) {
    uint64_t state = UINT64_C(0x123456789abcdef0), sum = 0;
    for (unsigned i = 0; i < BATCH6_SCALAR_TRIPS; ++i) {
        state ^= state << 13; state ^= state >> 7; state ^= state << 17;
        unsigned x = i % 4 == 0 ? 0 : i % 4 == 1 ? 1 :
                     i % 4 == 2 ? UINT32_MAX : (unsigned)state;
        unsigned y = (unsigned)(state >> 32), out;
        sum += b6_bits(x,y);
        sum += b6_sum(x,y,y);
        sum += b6_sub(x,y);
        sum += b6_range(x);
        sum += b6_even(state);
        sum += b6_shared(x,&out);
        sum += out;
        sum += b6_masked(x);
        sum += b6_select(x,y);
        sum += b6_signed_select((int)(x & 0x7fffffffu) - 0x40000000);
        sum += b6_trip(state);
        sum += b6_guard(x);
        sum += b6_lowone(x);
        sum += b6_boolor(state);
    }
    printf("%016llx\n", (unsigned long long)sum);
    return 0;
}
