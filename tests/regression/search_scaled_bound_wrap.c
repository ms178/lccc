/* Load-search affine bounds must recur in the unsigned 64-bit domain,
 * including runtime initialization, zero searches and wrap at ULONG_MAX. */
#include <stdint.h>
#include <limits.h>
#include <stdio.h>

__attribute__((noinline)) static unsigned long search(const uint64_t *p, unsigned long i, unsigned long limit) {
    while (!p[i & 7UL]) {
        if (((i + 1UL) << 6) >= limit) return i;
        ++i;
    }
    return i;
}
__attribute__((noinline)) static unsigned long reference(const volatile uint64_t *p, unsigned long i, unsigned long limit) {
    while (!p[i & 7UL]) {
        if (((i + 1UL) << 6) >= limit) return i;
        ++i;
    }
    return i;
}
int main(void) {
    uint64_t p[8] = {0,0,0,0,0,1,0,0};
    unsigned long limits[] = {0,1,63,64,65,127,128,511,512,513,ULONG_MAX-64,ULONG_MAX-1,ULONG_MAX};
    unsigned long starts[] = {0,1,4,5,6,7,8,15,ULONG_MAX>>6,(ULONG_MAX>>6)-1,ULONG_MAX-7,ULONG_MAX-1,ULONG_MAX};
    for (unsigned seed=0;seed<128;++seed) {
        for (unsigned k=0;k<8;++k) p[k]=0;
        p[seed&7]=1ULL << (seed&63);
        for (unsigned a=0;a<sizeof starts/sizeof starts[0];++a)
            for (unsigned b=0;b<sizeof limits/sizeof limits[0];++b)
                if (search(p,starts[a],limits[b]) != reference(p,starts[a],limits[b])) return 1;
    }
    puts("scaled search bounds: ok");
    return 0;
}
