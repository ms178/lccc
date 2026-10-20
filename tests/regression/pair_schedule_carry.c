/* Distance-two dword schedules: exact/tail/zero loops, overflow, unaligned
 * eight-byte pairs and a protected page immediately after the last word. */
#include <stdint.h>
#include <stdio.h>
#include <sys/mman.h>
#include <unistd.h>
#define R(x,n) (((x)>>(n))|((x)<<(32-(n))))
#define S0(x) (R(x,7)^R(x,18)^((x)>>3))
#define S1(x) (R(x,17)^R(x,19)^((x)>>10))
#define SCHEDULE(N) \
__attribute__((noinline)) static void schedule##N(uint32_t *p) { \
    uint32_t m[(N)>0?(N):1]; \
    for (int j=0;j<(N);++j) m[j]=p[j]; \
    for (int i=16;i<(N);++i) m[i]=S1(m[i-2])+m[i-7]+S0(m[i-15])+m[i-16]; \
    for (int j=0;j<(N);++j) p[j]=m[j]; \
}
SCHEDULE(0) SCHEDULE(16) SCHEDULE(17) SCHEDULE(18) SCHEDULE(19)
SCHEDULE(32) SCHEDULE(63) SCHEDULE(64) SCHEDULE(65)
__attribute__((noinline)) static void reference(volatile uint32_t *m, unsigned n) {
    for (unsigned i=16;i<n;++i) m[i]=S1(m[i-2])+m[i-7]+S0(m[i-15])+m[i-16];
}
int main(void) {
    long page=sysconf(_SC_PAGESIZE);
    if (page<1024) return 1;
    unsigned char *mem=mmap(0,(unsigned long)page*2,PROT_READ|PROT_WRITE,MAP_PRIVATE|MAP_ANONYMOUS,-1,0);
    if (mem==MAP_FAILED) return 2;
    if (mprotect(mem+page,(unsigned long)page,PROT_NONE)) return 3;
    void (*f[])(uint32_t *)={schedule0,schedule16,schedule17,schedule18,schedule19,schedule32,schedule63,schedule64,schedule65};
    unsigned ns[]={0,16,17,18,19,32,63,64,65};
    uint32_t expected[65];
    uint32_t seed=0x7fffffffU;
    for (unsigned round=0;round<128;++round) {
        for (unsigned k=0;k<9;++k) {
            unsigned n=ns[k];
            uint32_t *m=(uint32_t *)(mem+page-n*4);
            for (unsigned i=0;i<n;++i) {
                seed=seed*1664525U+1013904223U;
                m[i]=expected[i]=seed;
            }
            f[k](m);reference(expected,n);
            for (unsigned i=0;i<n;++i) if (m[i]!=expected[i]) return 4;
        }
    }
    if (munmap(mem,(unsigned long)page*2)) return 5;
    puts("pair schedule carry: ok");
    return 0;
}
