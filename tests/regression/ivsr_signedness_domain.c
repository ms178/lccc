/* Signed interpretation after unsigned wrap is a discontinuous address map.
 * The target's two's-complement conversion is deliberate, not signed overflow.
 * All resulting subscripts are within the same small array object. */
#include <stdint.h>
#include <stdio.h>
__attribute__((noinline)) static unsigned scan32(const unsigned *p, uint32_t i, uint32_t stop) {
    unsigned s=0;
    do { s+=p[(int32_t)i]; ++i; } while(i!=stop);
    return s;
}
__attribute__((noinline)) static unsigned scan64(const unsigned *p, uint64_t i, uint64_t stop) {
    unsigned s=0;
    do { s+=p[(int64_t)i]; ++i; } while(i!=stop);
    return s;
}
__attribute__((noinline)) static unsigned affine32(const unsigned *p, uint32_t i, uint32_t stop) {
    unsigned s=0;
    do { s+=p[i+UINT32_C(0x80000000)]; ++i; } while(i!=stop);
    return s;
}
int main(void) {
    unsigned a[8]={17,29,43,71,97,131,173,211};
    for (unsigned n=1;n<=6;++n) {
        unsigned want=0;
        for (unsigned k=0;k<n;++k) want+=a[k];
        if (scan32(a+2,UINT32_MAX-1,n-2)!=want) return 1;
        if (scan64(a+2,UINT64_MAX-1,(uint64_t)n-2)!=want) return 2;
        if (affine32(a,UINT32_C(0x80000000),UINT32_C(0x80000000)+n)!=want) return 3;
    }
    puts("ivsr_signedness_domain: OK");
    return 0;
}
