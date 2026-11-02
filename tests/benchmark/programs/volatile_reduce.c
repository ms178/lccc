/* Single-threaded volatile reads cannot be CSE'd or widened. Independent
 * unsigned accumulator chains preserve those reads but expose scalar ILP. */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>

#ifndef VOLATILE_REDUCE_TRIPS
#define VOLATILE_REDUCE_TRIPS 2000000
#endif

__thread volatile uint64_t volatile_reduce_value = UINT64_C(0x9e3779b97f4a7c15);

__attribute__((noinline)) uint64_t volatile_sum(uint64_t n, uint64_t seed) {
    for (uint64_t i=0;i<n;++i) seed += volatile_reduce_value;
    return seed;
}
__attribute__((noinline)) uint64_t volatile_product(uint64_t n, uint64_t seed) {
    for (uint64_t i=0;i<n;++i) seed *= volatile_reduce_value;
    return seed;
}

int main(int argc, char **argv) {
    uint64_t n=argc>1 ? strtoull(argv[1],0,0) : VOLATILE_REDUCE_TRIPS;
    uint64_t value=volatile_reduce_value, power=1, base=value;
    for (uint64_t k=n;k;k>>=1) {
        if (k&1) power *= base;
        base *= base;
    }
    uint64_t sum=volatile_sum(n,7), product=volatile_product(n,3);
    printf("%016llx %016llx\n", (unsigned long long)sum,(unsigned long long)product);
    return sum != 7+n*value || product != 3*power;
}
