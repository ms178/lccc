/* Build with the exact reference GCC; dynamically load each diagnostic kernel.
 * Internal CPU/wall timers exclude loader/process startup and oracle setup. */
#define _POSIX_C_SOURCE 200809L
#include <dlfcn.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <time.h>

typedef uint64_t (*Kernel)(volatile void *, uint64_t, uint64_t,
    uint64_t, uint64_t, uint64_t, uint64_t, uint64_t, uint64_t, uint64_t, uint64_t);
static uint64_t ns(clockid_t clock) {
    struct timespec t;
    if (clock_gettime(clock, &t)) exit(9);
    return (uint64_t)t.tv_sec*UINT64_C(1000000000)+(uint64_t)t.tv_nsec;
}
int main(int argc, char **argv) {
    if (argc != 8) return 1;
    uint64_t n = strtoull(argv[2], 0, 0), calls = strtoull(argv[3], 0, 0);
    unsigned kind = (unsigned)strtoul(argv[4], 0, 0);
    unsigned width = (unsigned)strtoul(argv[5], 0, 0);
    unsigned pressure = (unsigned)strtoul(argv[6], 0, 0);
    unsigned address = (unsigned)strtoul(argv[7], 0, 0);
    if (!calls || kind > 4 || (width != 32 && width != 64) || pressure > 1 || address > 2) return 2;
    void *library = dlopen(argv[1], RTLD_NOW);
    if (!library) { fprintf(stderr, "%s\n", dlerror()); return 3; }
    Kernel kernel = (Kernel)dlsym(library, "reduce");
    /* Distinct correctly typed objects: reading a uint64_t through uint32_t*
     * would violate C effective-type rules even across an opaque DSO call. */
    volatile uint32_t value32 = (uint32_t)UINT64_C(0x9e3779b97f4a7c15);
    volatile uint64_t value64 = UINT64_C(0x9e3779b97f4a7c15);
    volatile void *pointer = width == 32 ? (volatile void *)&value32 : (volatile void *)&value64;
    if (!kernel) return 4;
    if (address) {
        pointer = dlsym(library, "volatile_matrix_value");
        if (!pointer) return 5;
    }
    uint64_t mask = width == 32 ? UINT32_MAX : UINT64_MAX;
    uint64_t v = width == 32 ? value32 : value64, expected[8];
    for (uint64_t j=0;j<8;++j) {
        uint64_t result = 7+j;
        /* Logarithmic modular oracle; independent of the grouped loop. */
        if (kind == 0) result += n*v;
        else if (kind == 1) {
            uint64_t power=1, base=v;
            for (uint64_t k=n;k;k>>=1) { if (k&1) power *= base; base *= base; }
            result *= power;
        } else if (kind == 2 && n) result &= v;
        else if (kind == 3 && n) result |= v;
        else if (kind == 4 && (n&1)) result ^= v;
        result &= mask;
        if (pressure) for (uint64_t k=1;k<=8;++k) result ^= 7+j+k;
        expected[j] = result;
    }
    uint64_t hash=0, cpu=ns(CLOCK_PROCESS_CPUTIME_ID), wall=ns(CLOCK_MONOTONIC);
    for (uint64_t i=0;i<calls;++i) {
        uint64_t seed=7+(i&7);
        uint64_t actual=kernel(pointer,n,seed,seed+1,seed+2,seed+3,seed+4,seed+5,seed+6,seed+7,seed+8);
        if (actual != expected[i&7]) return 6;
        hash += actual;
    }
    wall=ns(CLOCK_MONOTONIC)-wall;
    cpu=ns(CLOCK_PROCESS_CPUTIME_ID)-cpu;
    printf("%016llx %llu %llu\n", (unsigned long long)hash,
        (unsigned long long)cpu, (unsigned long long)wall);
    return dlclose(library) != 0;
}
