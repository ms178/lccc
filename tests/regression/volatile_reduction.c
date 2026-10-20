/* Volatile scalar interleaving contracts: modular arithmetic, arbitrary seeds,
 * every cleanup size, exact widths, and zero-trip invalid pointers. */
#include <stdint.h>
#include <stdio.h>

#define REDUCTION(NAME, TYPE, OP) \
__attribute__((noinline)) TYPE NAME(volatile TYPE *p, uint64_t n, TYPE seed) { \
    for (uint64_t i = 0; i < n; ++i) seed OP *p; \
    return seed; \
}
REDUCTION(add32, uint32_t, +=)
REDUCTION(mul32, uint32_t, *=)
REDUCTION(and32, uint32_t, &=)
REDUCTION(or32, uint32_t, |=)
REDUCTION(xor32, uint32_t, ^=)
REDUCTION(add64, uint64_t, +=)
REDUCTION(mul64, uint64_t, *=)
REDUCTION(and64, uint64_t, &=)
REDUCTION(or64, uint64_t, |=)
REDUCTION(xor64, uint64_t, ^=)

__thread volatile uint64_t volatile_reduction_slots[3];
__attribute__((noinline)) uint64_t tls_product(uint64_t n, uint64_t seed) {
    for (uint64_t i = 0; i < n; ++i) seed *= volatile_reduction_slots[1];
    return seed;
}

__attribute__((noinline)) uint64_t live_out(volatile uint64_t *p, uint64_t n, uint64_t seed) {
    uint64_t i;
    for (i = 0; i < n; ++i) seed += *p;
    return seed ^ i;
}

int main(void) {
    static const uint64_t values[] = {0, 1, 3, UINT64_C(0x9e3779b97f4a7c15), UINT64_MAX};
    static const uint64_t counts[] = {0,1,2,3,4,5,6,7,8,9,15,16,17,31,32,33,63,64,65,1023};
    static const uint64_t seeds[] = {0,1,7,UINT64_C(0xdeadbeef01234567),UINT64_MAX};
    volatile uint32_t a;
    volatile uint64_t b;
    /* Zero iterations must not turn the main group into a speculative read. */
    volatile uint64_t zero = 0;
    if (add32(0,zero,7) != 7 || mul64(0,zero,3) != 3) return 1;
    uint64_t hash = 0;
    for (unsigned v=0; v<sizeof(values)/sizeof(values[0]); ++v) {
        a = (uint32_t)values[v]; b = values[v];
        volatile_reduction_slots[1] = values[v];
        for (unsigned j=0; j<sizeof(counts)/sizeof(counts[0]); ++j) {
            for (unsigned s=0; s<sizeof(seeds)/sizeof(seeds[0]); ++s) {
                uint64_t n=counts[j], seed=seeds[s];
                uint64_t sum=seed+n*values[v], product=seed, x=seed;
                uint64_t both=n ? seed&values[v] : seed;
                uint64_t either=n ? seed|values[v] : seed;
                for (uint64_t i=0;i<n;++i) product *= values[v];
                if (n&1) x ^= values[v];
                if (add32(&a,n,(uint32_t)seed)!=(uint32_t)sum ||
                    mul32(&a,n,(uint32_t)seed)!=(uint32_t)product ||
                    and32(&a,n,(uint32_t)seed)!=(uint32_t)both ||
                    or32(&a,n,(uint32_t)seed)!=(uint32_t)either ||
                    xor32(&a,n,(uint32_t)seed)!=(uint32_t)x ||
                    add64(&b,n,seed)!=sum || mul64(&b,n,seed)!=product ||
                    and64(&b,n,seed)!=both || or64(&b,n,seed)!=either ||
                    xor64(&b,n,seed)!=x || tls_product(n,seed)!=product ||
                    live_out(&b,n,seed)!=(sum^n)) return 2;
                hash ^= sum + product + both + either + x;
            }
        }
    }
    printf("%016llx\n", (unsigned long long)hash);
    return 0;
}
