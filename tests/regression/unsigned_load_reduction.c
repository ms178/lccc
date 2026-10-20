/* Exact-trip load-only unsigned reductions: wraps, tails, unaligned byte
 * assembly, and the final access adjacent to a protected page. */
#include <stdint.h>
#include <stdio.h>
#include <sys/mman.h>
#include <unistd.h>

#define REDUCE(N) \
__attribute__((noinline)) static uint32_t sum##N(const unsigned char *p, uint32_t acc) { \
    for (unsigned long i = 0; i < (N); ++i) { \
        uint32_t x = (uint32_t)p[8*i] | ((uint32_t)p[8*i+1] << 8) | \
                     ((uint32_t)p[8*i+2] << 16) | ((uint32_t)p[8*i+3] << 24); \
        acc += x ^ (uint32_t)i; \
    } \
    return acc; \
}
__attribute__((noinline)) static uint32_t sum0(const unsigned char *p, uint32_t acc) { (void)p; return acc; }
REDUCE(1) REDUCE(2) REDUCE(4) REDUCE(63) REDUCE(64) REDUCE(65) REDUCE(66) REDUCE(68)

__attribute__((noinline)) static uint32_t reference(const volatile unsigned char *p, unsigned n, uint32_t acc) {
    for (unsigned i = 0; i < n; ++i) {
        uint32_t x = (uint32_t)p[8*i] | ((uint32_t)p[8*i+1] << 8) |
                     ((uint32_t)p[8*i+2] << 16) | ((uint32_t)p[8*i+3] << 24);
        acc += x ^ i;
    }
    return acc;
}
__attribute__((noinline)) static uint64_t wide(const uint64_t *p, uint64_t acc) {
    for (unsigned i=0; i<64; ++i) acc += p[i];
    return acc;
}
int main(void) {
    long page = sysconf(_SC_PAGESIZE);
    if (page < 1024) return 1;
    unsigned char *mem = mmap(0, (unsigned long)page*2, PROT_READ|PROT_WRITE, MAP_PRIVATE|MAP_ANONYMOUS, -1, 0);
    if (mem == MAP_FAILED) return 2;
    if (mprotect(mem+page, (unsigned long)page, PROT_NONE)) return 3;
    uint32_t (*f[])(const unsigned char *,uint32_t) = {sum0,sum1,sum2,sum4,sum63,sum64,sum65,sum66,sum68};
    unsigned ns[] = {0,1,2,4,63,64,65,66,68};
    for (unsigned seed=0; seed<100; ++seed) {
        for (unsigned j=0; j<9; ++j) {
            unsigned n=ns[j];
            /* The last valid byte ends exactly at the protection boundary. */
            unsigned char *p=mem+page-(n ? 8*(n-1)+4 : 0);
            for (unsigned i=0; i<(n ? 8*(n-1)+4 : 0); ++i) p[i]=(unsigned char)(i*37+seed*113);
            uint32_t acc=0xffffffffu-seed*1234567u;
            if (f[j](p,acc) != reference(p,n,acc)) return 4;
        }
        uint64_t *p=(uint64_t *)(mem+page-64*8);
        uint64_t expected=~(uint64_t)seed;
        for (unsigned i=0;i<64;++i) {p[i]=(~(uint64_t)0-i)*((uint64_t)seed+1); expected+=p[i];}
        if (wide(p,~(uint64_t)seed)!=expected) return 5;
    }
    if (munmap(mem,(unsigned long)page*2)) return 6;
    puts("unsigned load reduction: ok");
    return 0;
}
