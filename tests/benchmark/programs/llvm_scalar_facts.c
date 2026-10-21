/* Synthetic throughput bundle, not a workload-speed claim. Keep the exact
 * noinline entry points shared with the native semantic/mechanism fixture. */
#define SCALAR_FACTS_BENCH 1
#include "../../regression/llvm_scalar_facts.c"
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
        sum += sf_bits(x,y);
        sum += sf_sum(x,y,y);
        sum += sf_sub(x,y);
        sum += sf_range(x);
        sum += sf_even(state);
        sum += sf_shared(x,&out);
        sum += out;
        sum += sf_masked(x);
        sum += sf_select(x,y);
        sum += sf_signed_select((int)(x & 0x7fffffffu) - 0x40000000);
        sum += sf_trip(state);
        sum += sf_guard(x);
        sum += sf_lowone(x);
        sum += sf_boolor(state);
    }
    printf("%016llx\n", (unsigned long long)sum);
    return 0;
}
